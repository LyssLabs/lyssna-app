//! Sign-in to the user's Lyssna account (lyrics submissions, reviews and
//! notifications). The browser signs in on lyssna.org and hands a one-time
//! code back to a loopback listener; the code is exchanged with a PKCE
//! verifier for an access and refresh token pair kept in the credential store.

use std::{
    net::TcpListener,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use reqwest::{StatusCode, blocking::Client};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use url::Url;

use crate::auth::{challenge, random_string, read_target, reply};
use crate::credential::{Entry, Error as CredentialError};
use crate::model::Track;

pub const API: &str = "https://api.lyssna.org";
const WEB: &str = "https://lyssna.org";
const CALLBACK_PATH: &str = "/lyssna";
const CREDENTIAL_SERVICE: &str = "Lyssna/Account";
const CREDENTIAL_USER: &str = "tokens";

#[derive(Clone, Serialize, Deserialize)]
struct Tokens {
    access_token: String,
    access_expires_at: i64,
    refresh_token: String,
    refresh_expires_at: i64,
}

#[derive(Deserialize)]
struct TokenPair {
    access_token: String,
    access_expires_at: String,
    refresh_token: String,
    refresh_expires_at: String,
}

impl TryFrom<TokenPair> for Tokens {
    type Error = anyhow::Error;
    fn try_from(pair: TokenPair) -> Result<Self> {
        let unix = |text: &str| -> Result<i64> {
            Ok(OffsetDateTime::parse(text, &Rfc3339)
                .context("Lyssna sent an unreadable token expiry")?
                .unix_timestamp())
        };
        Ok(Self {
            access_expires_at: unix(&pair.access_expires_at)?,
            refresh_expires_at: unix(&pair.refresh_expires_at)?,
            access_token: pair.access_token,
            refresh_token: pair.refresh_token,
        })
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Account {
    pub id: String,
    pub username: String,
    pub role: String,
    pub reputation: i64,
    pub tier_name: String,
    pub email_verified: bool,
}

#[derive(Deserialize)]
struct MeResponse {
    user: MeUser,
    tier: MeTier,
}
#[derive(Deserialize)]
struct MeUser {
    id: String,
    username: String,
    role: String,
    reputation: i64,
    email_verified: bool,
}
#[derive(Deserialize)]
struct MeTier {
    name: String,
}

#[derive(Deserialize)]
struct ApiError {
    #[serde(default)]
    error: Option<String>,
    message: Option<String>,
}

// Refresh tokens rotate on use, so two refreshes racing would sign the user out.
static TOKENS: Mutex<()> = Mutex::new(());

fn now() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

fn entry() -> Result<Entry> {
    Entry::new(CREDENTIAL_SERVICE, CREDENTIAL_USER).context("Credential store unavailable")
}

fn load() -> Result<Option<Tokens>> {
    match entry()?.get_password() {
        Ok(text) => Ok(serde_json::from_str(&text).ok()),
        Err(CredentialError::NoEntry) => Ok(None),
        Err(error) => Err(anyhow::anyhow!("Cannot read saved Lyssna sign-in: {error}")),
    }
}

fn save(tokens: &Tokens) -> Result<()> {
    entry()?
        .set_password(&serde_json::to_string(tokens)?)
        .context("Cannot save Lyssna sign-in")
}

fn forget() -> Result<()> {
    match entry()?.delete_credential() {
        Ok(()) | Err(CredentialError::NoEntry) => Ok(()),
        Err(error) => Err(anyhow::anyhow!(
            "Cannot remove saved Lyssna sign-in: {error}"
        )),
    }
}

fn failure(response: reqwest::blocking::Response) -> anyhow::Error {
    let status = response.status();
    let message = response
        .json::<ApiError>()
        .ok()
        .and_then(|body| body.message)
        .unwrap_or_else(|| format!("Lyssna answered {status}"));
    anyhow::anyhow!("{message}")
}

fn request_tokens(client: &Client, body: &serde_json::Value) -> Result<Tokens> {
    let response = client
        .post(format!("{API}/v1/auth/token"))
        .json(body)
        .send()
        .context("Cannot reach Lyssna")?;
    if !response.status().is_success() {
        return Err(failure(response));
    }
    response.json::<TokenPair>()?.try_into()
}

fn sign_in_url(redirect_uri: &str, verifier: &str, state: &str) -> Result<Url> {
    let mut url = Url::parse(&format!("{WEB}/login"))?;
    url.query_pairs_mut().extend_pairs([
        ("client", "desktop"),
        ("redirect_uri", redirect_uri),
        ("code_challenge", &challenge(verifier)),
        ("state", state),
    ]);
    Ok(url)
}

/// The code from a loopback callback, after checking its path and state.
fn callback_code(target: &str, state: &str) -> Result<String> {
    let url = Url::parse(&format!("http://127.0.0.1{target}"))?;
    ensure!(url.path() == CALLBACK_PATH, "Unexpected callback path");
    let field = |name: &str| -> Result<Option<String>> {
        let values: Vec<_> = url.query_pairs().filter(|(key, _)| key == name).collect();
        ensure!(values.len() <= 1, "Duplicate callback parameter");
        Ok(values.first().map(|(_, value)| value.to_string()))
    };
    ensure!(
        field("state")?.as_deref() == Some(state),
        "Sign-in state did not match"
    );
    let code = field("code")?.context("Lyssna did not return a sign-in code")?;
    ensure!(!code.is_empty(), "Lyssna returned an empty sign-in code");
    Ok(code)
}

fn wait_for_code(listener: &TcpListener, state: &str, cancelled: &AtomicBool) -> Result<String> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline {
        ensure!(!cancelled.load(Ordering::Relaxed), "Sign-in cancelled");
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let Ok(target) = read_target(&mut stream) else {
                    reply(&mut stream, "400 Bad Request", "Invalid callback request.");
                    continue;
                };
                match callback_code(&target, state) {
                    Ok(code) => {
                        reply(
                            &mut stream,
                            "200 OK",
                            "Signed in. Return to Lyssna to finish.",
                        );
                        return Ok(code);
                    }
                    // A stray local request must not end the real sign-in.
                    Err(_) => reply(&mut stream, "400 Bad Request", "Sign-in callback rejected."),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50))
            }
            Err(error) => return Err(error.into()),
        }
    }
    bail!("Lyssna sign-in timed out after five minutes. Start it again from Settings.")
}

pub fn sign_in(client: &Client, cancelled: &AtomicBool) -> Result<Account> {
    let verifier = random_string()?;
    let state = random_string()?;
    let listener = TcpListener::bind("127.0.0.1:0").context("Cannot open a local sign-in port")?;
    let redirect_uri = format!(
        "http://127.0.0.1:{}{CALLBACK_PATH}",
        listener.local_addr()?.port()
    );
    let url = sign_in_url(&redirect_uri, &verifier, &state)?;
    webbrowser::open(url.as_str()).context("Cannot open the browser to sign in to Lyssna")?;
    let code = wait_for_code(&listener, &state, cancelled)?;
    let tokens = request_tokens(
        client,
        &serde_json::json!({
            "grant_type": "authorization_code",
            "code": code,
            "code_verifier": verifier,
            "redirect_uri": redirect_uri,
        }),
    )?;
    {
        let _guard = TOKENS.lock().unwrap_or_else(|error| error.into_inner());
        save(&tokens)?;
    }
    account(client)?.context("Lyssna did not accept the new sign-in")
}

/// A usable access token, refreshing it first when it is about to expire.
/// `None` when signed out or the saved sign-in is no longer accepted.
fn access_token(client: &Client) -> Result<Option<String>> {
    let _guard = TOKENS.lock().unwrap_or_else(|error| error.into_inner());
    let Some(tokens) = load()? else {
        return Ok(None);
    };
    if tokens.access_expires_at - now() > 60 {
        return Ok(Some(tokens.access_token));
    }
    if tokens.refresh_expires_at <= now() {
        forget()?;
        return Ok(None);
    }
    let response = client
        .post(format!("{API}/v1/auth/token"))
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": tokens.refresh_token,
        }))
        .send()
        .context("Cannot reach Lyssna")?;
    if matches!(
        response.status(),
        StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED
    ) {
        forget()?;
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(failure(response));
    }
    let fresh: Tokens = response.json::<TokenPair>()?.try_into()?;
    save(&fresh)?;
    Ok(Some(fresh.access_token))
}

/// The signed-in account, or `None` when signed out.
pub fn account(client: &Client) -> Result<Option<Account>> {
    let Some(token) = access_token(client)? else {
        return Ok(None);
    };
    let response = client
        .get(format!("{API}/v1/me"))
        .bearer_auth(token)
        .send()
        .context("Cannot reach Lyssna")?;
    if response.status() == StatusCode::UNAUTHORIZED {
        let _guard = TOKENS.lock().unwrap_or_else(|error| error.into_inner());
        forget()?;
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(failure(response));
    }
    let me: MeResponse = response.json()?;
    Ok(Some(Account {
        id: me.user.id,
        username: me.user.username,
        role: me.user.role,
        reputation: me.user.reputation,
        tier_name: me.tier.name,
        email_verified: me.user.email_verified,
    }))
}

/// What Lyssna made of an uploaded TTML.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Submission {
    pub id: String,
    pub song_title: String,
    /// `new` or `edit`.
    pub kind: String,
    /// `approved` when the submitter's role skips review, otherwise `pending`.
    pub status: String,
}

fn submission_body(ttml: &str, track: &Track) -> Result<serde_json::Value> {
    let Some(id) = crate::lyssna_lyrics::spotify_id(track) else {
        bail!("Only Spotify songs can be submitted to Lyssna");
    };
    let mut body = serde_json::json!({
        "ttml": ttml,
        "links": [{ "platform": "spotify", "external_id": id }],
        "title": track.name,
        "artists": track.artists.iter().map(|artist| &artist.name).collect::<Vec<_>>(),
    });
    if !track.album.name.is_empty() {
        body["album"] = track.album.name.clone().into();
    }
    Ok(body)
}

fn submit_failure(response: reqwest::blocking::Response) -> anyhow::Error {
    let status = response.status();
    let body = response.json::<ApiError>().unwrap_or(ApiError {
        error: None,
        message: None,
    });
    let known = match body.error.as_deref() {
        Some("unverified") => Some("Confirm your email on lyssna.org before submitting lyrics"),
        Some("pending_exists") => Some("This song already has lyrics waiting for review"),
        Some("own_open_edit") => Some("You already have an open submission for this song"),
        Some("no_changes") => Some("These lyrics match what Lyssna already has"),
        _ => None,
    };
    match (known, body.message) {
        (Some(text), _) => anyhow::anyhow!("{text}"),
        (None, Some(message)) => anyhow::anyhow!("{message}"),
        (None, None) => anyhow::anyhow!("Lyssna answered {status}"),
    }
}

/// Uploads a TTML for the song, linked by its Spotify ID.
pub fn submit(client: &Client, ttml: &str, track: &Track) -> Result<Submission> {
    let body = submission_body(ttml, track)?;
    let Some(token) = access_token(client)? else {
        bail!("Sign in to Lyssna in Settings to submit lyrics");
    };
    let response = client
        .post(format!("{API}/v1/submissions"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .context("Cannot reach Lyssna")?;
    if !response.status().is_success() {
        return Err(submit_failure(response));
    }
    Ok(response.json()?)
}

pub fn signed_in() -> bool {
    matches!(load(), Ok(Some(_)))
}

pub fn sign_out(client: &Client) -> Result<()> {
    let _guard = TOKENS.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(tokens) = load()? {
        // Best effort: the server session also expires on its own.
        let _ = client
            .post(format!("{API}/v1/auth/logout"))
            .bearer_auth(&tokens.access_token)
            .json(&serde_json::json!({}))
            .send();
    }
    forget()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_url_carries_the_desktop_request() {
        let url = sign_in_url("http://127.0.0.1:5000/lyssna", "verifier", "state").unwrap();
        assert_eq!(url.host_str(), Some("lyssna.org"));
        assert_eq!(url.path(), "/login");
        let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        assert!(query.contains(&("client".into(), "desktop".into())));
        assert!(query.contains(&("redirect_uri".into(), "http://127.0.0.1:5000/lyssna".into())));
        assert!(query.contains(&("code_challenge".into(), challenge("verifier"))));
        assert!(query.contains(&("state".into(), "state".into())));
    }

    #[test]
    fn callback_requires_matching_state_and_path() {
        assert_eq!(
            callback_code("/lyssna?code=abc&state=s1", "s1").unwrap(),
            "abc"
        );
        assert!(callback_code("/lyssna?code=abc&state=other", "s1").is_err());
        assert!(callback_code("/other?code=abc&state=s1", "s1").is_err());
        assert!(callback_code("/lyssna?code=abc&code=def&state=s1", "s1").is_err());
        assert!(callback_code("/lyssna?state=s1", "s1").is_err());
        assert!(callback_code("/lyssna?code=&state=s1", "s1").is_err());
    }

    #[test]
    fn token_expiry_reads_go_timestamps() {
        let tokens: Tokens = TokenPair {
            access_token: "a".into(),
            access_expires_at: "2026-11-18T10:00:00.123456789Z".into(),
            refresh_token: "r".into(),
            refresh_expires_at: "2026-12-18T10:00:00+01:00".into(),
        }
        .try_into()
        .unwrap();
        assert_eq!(
            tokens.refresh_expires_at - tokens.access_expires_at,
            30 * 86_400 - 3_600
        );
    }

    #[test]
    fn me_reads_the_username() {
        let me: MeResponse = serde_json::from_str(
            r#"{"user":{"id":"u1","username":"Arashii","role":"admin","reputation":5,
                "email_verified":true,"avatar_source":"discord"},"tier":{"level":0,"name":"New"}}"#,
        )
        .unwrap();
        assert_eq!(me.user.username, "Arashii");
        assert_eq!(me.tier.name, "New");
    }

    #[test]
    fn submission_links_the_spotify_song_and_names_it() {
        let track = Track {
            id: Some("4cOdK2wGLETKBW3PvgPWqT".into()),
            name: "Uprising".into(),
            artists: vec![crate::model::Artist {
                name: "Muse".into(),
                ..Default::default()
            }],
            ..Track::default()
        };
        let body = submission_body("<tt/>", &track).unwrap();
        assert_eq!(body["links"][0]["platform"], "spotify");
        assert_eq!(body["links"][0]["external_id"], "4cOdK2wGLETKBW3PvgPWqT");
        assert_eq!(body["title"], "Uprising");
        assert_eq!(body["artists"][0], "Muse");
        assert!(body.get("album").is_none());

        let local = Track {
            is_local: true,
            ..track
        };
        assert!(submission_body("<tt/>", &local).is_err());
    }

    #[test]
    fn submission_reads_what_lyssna_made_of_it() {
        let submission: Submission = serde_json::from_str(
            r#"{"id":"s1","song_id":"x","song_title":"Uprising","kind":"new","status":"pending","priority":false}"#,
        )
        .unwrap();
        assert_eq!(submission.status, "pending");
        assert_eq!(submission.kind, "new");
    }
}

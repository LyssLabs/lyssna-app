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

const API: &str = "https://api.lyssna.org";
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
    pub display_name: String,
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
    display_name: String,
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
        display_name: me.user.display_name,
        role: me.user.role,
        reputation: me.user.reputation,
        tier_name: me.tier.name,
        email_verified: me.user.email_verified,
    }))
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
}

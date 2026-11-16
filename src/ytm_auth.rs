//! The YouTube Music session: the youtube.com cookies a signed-in browser
//! holds, kept only in the platform credential store. They grant access to
//! the whole Google account, so they are never logged or written to disk.

use anyhow::{Context, Result, ensure};

use crate::credential as keyring;

const SERVICE: &str = "Lyssna/YouTubeMusic";
const COUNT_USER: &str = "session";
/// Windows keeps at most 2560 bytes per credential, stored as UTF-16.
const CHUNK: usize = 1200;
const MAX_CHUNKS: usize = 16;

/// The cookies YouTube Music's web client signs requests with; the rest of
/// a browser's youtube.com cookies are preferences and tracking.
const KEPT: &[&str] = &[
    "SID",
    "HSID",
    "SSID",
    "APISID",
    "SAPISID",
    "LOGIN_INFO",
    "__Secure-1PSID",
    "__Secure-3PSID",
    "__Secure-1PAPISID",
    "__Secure-3PAPISID",
    "__Secure-1PSIDTS",
    "__Secure-3PSIDTS",
    "__Secure-1PSIDCC",
    "__Secure-3PSIDCC",
    "SIDCC",
    "VISITOR_INFO1_LIVE",
    "VISITOR_PRIVACY_METADATA",
    "PREF",
];

#[derive(Clone, PartialEq, Eq)]
pub struct Session {
    cookies: Vec<(String, String)>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<_> = self.cookies.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("Session").field("cookies", &names).finish()
    }
}

fn plain(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b';' | b','))
}

impl Session {
    /// Keeps the signing cookies, last value of each name winning. None
    /// unless the ones that prove a sign-in are present.
    pub fn from_cookies<'a>(cookies: impl IntoIterator<Item = (&'a str, &'a str)>) -> Option<Self> {
        let mut kept: Vec<(String, String)> = Vec::new();
        for (name, value) in cookies {
            if !KEPT.contains(&name) || !plain(value) {
                continue;
            }
            kept.retain(|(known, _)| known != name);
            kept.push((name.to_owned(), value.to_owned()));
        }
        kept.sort_by_key(|(name, _)| KEPT.iter().position(|kept| kept == name));
        let has = |name: &str| kept.iter().any(|(known, _)| known == name);
        let signed_in =
            (has("SAPISID") || has("__Secure-3PAPISID")) && (has("SID") || has("__Secure-3PSID"));
        signed_in.then_some(Self { cookies: kept })
    }

    /// A Netscape cookies.txt export, as browser extensions write it.
    pub fn from_cookies_txt(contents: &str) -> Option<Self> {
        Self::from_cookies(contents.lines().filter_map(|line| {
            let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
            if line.starts_with('#') {
                return None;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            let [domain, _, _, _, _, name, value] = fields[..] else {
                return None;
            };
            let domain = domain.trim_start_matches('.');
            (domain == "youtube.com" || domain.ends_with(".youtube.com"))
                .then_some((name, value.trim_end()))
        }))
    }

    pub fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, value)| value.as_str())
    }

    fn parse_header(header: &str) -> Option<Self> {
        Self::from_cookies(header.split("; ").filter_map(|pair| pair.split_once('=')))
    }
}

fn entry(user: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(SERVICE, user)?)
}

fn chunks(header: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = header;
    while !rest.is_empty() {
        let mut end = rest.len().min(CHUNK);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        parts.push(&rest[..end]);
        rest = &rest[end..];
    }
    parts
}

pub fn save(session: &Session) -> Result<()> {
    let header = session.cookie_header();
    let parts = chunks(&header);
    ensure!(
        parts.len() <= MAX_CHUNKS,
        "The YouTube Music session is too large to save"
    );
    forget()?;
    for (index, part) in parts.iter().enumerate() {
        entry(&format!("{COUNT_USER}.{index}"))?
            .set_password(part)
            .context("Cannot save the YouTube Music sign-in to the secure credential store")?;
    }
    entry(COUNT_USER)?
        .set_password(&parts.len().to_string())
        .context("Cannot save the YouTube Music sign-in to the secure credential store")
}

pub fn load() -> Result<Option<Session>> {
    let count = match entry(COUNT_USER)?.get_password() {
        Ok(count) => count.parse::<usize>().unwrap_or(0).min(MAX_CHUNKS),
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(error) => return Err(error).context("Cannot read the YouTube Music sign-in"),
    };
    let mut header = String::new();
    for index in 0..count {
        match entry(&format!("{COUNT_USER}.{index}"))?.get_password() {
            Ok(part) => header.push_str(&part),
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(error) => return Err(error).context("Cannot read the YouTube Music sign-in"),
        }
    }
    Ok(Session::parse_header(&header))
}

pub fn signed_in() -> bool {
    matches!(load(), Ok(Some(_)))
}

pub fn forget() -> Result<()> {
    let ignore_missing = |result: std::result::Result<(), keyring::Error>| match result {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error),
    };
    ignore_missing(entry(COUNT_USER)?.delete_credential())?;
    for index in 0..MAX_CHUNKS {
        ignore_missing(entry(&format!("{COUNT_USER}.{index}"))?.delete_credential())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_in_cookies() -> Vec<(&'static str, &'static str)> {
        vec![
            ("YSC", "tracking"),
            ("SAPISID", "abc/def"),
            ("SID", "g.a000"),
            ("__Secure-3PAPISID", "abc/def"),
            ("PREF", "f6=40000000&tz=Australia.Sydney"),
        ]
    }

    #[test]
    fn keeps_only_the_signing_cookies() {
        let session = Session::from_cookies(signed_in_cookies()).unwrap();
        assert_eq!(
            session.cookie_header(),
            "SID=g.a000; SAPISID=abc/def; __Secure-3PAPISID=abc/def; PREF=f6=40000000&tz=Australia.Sydney"
        );
        assert_eq!(session.get("YSC"), None);
    }

    #[test]
    fn a_signed_out_browser_is_not_a_session() {
        assert!(Session::from_cookies([("PREF", "x"), ("VISITOR_INFO1_LIVE", "y")]).is_none());
        assert!(Session::from_cookies([("SAPISID", "a")]).is_none());
    }

    #[test]
    fn values_that_would_break_the_header_are_dropped() {
        let session = Session::from_cookies([
            ("SAPISID", "a; evil=1"),
            ("SID", "s"),
            ("__Secure-3PAPISID", "b"),
        ])
        .unwrap();
        assert_eq!(session.get("SAPISID"), None);
        assert_eq!(session.cookie_header(), "SID=s; __Secure-3PAPISID=b");
    }

    #[test]
    fn reads_youtube_cookies_from_a_cookies_txt_export() {
        let txt = "# Netscape HTTP Cookie File\n\
            .youtube.com\tTRUE\t/\tTRUE\t1790000000\tSAPISID\tabc\n\
            #HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1790000000\tSID\tsid\r\n\
            .google.com\tTRUE\t/\tTRUE\t1790000000\t__Secure-3PSID\tgoogle-only\n\
            .youtube.com\tTRUE\t/\tTRUE\t1790000000\tYSC\tskip\n";
        let session = Session::from_cookies_txt(txt).unwrap();
        assert_eq!(session.cookie_header(), "SID=sid; SAPISID=abc");
    }

    #[test]
    fn the_header_round_trips_through_saved_chunks() {
        let long = "x".repeat(900);
        let session = Session::from_cookies([
            ("SID", "s"),
            ("SAPISID", long.as_str()),
            ("LOGIN_INFO", long.as_str()),
        ])
        .unwrap();
        let header = session.cookie_header();
        let parts = chunks(&header);
        assert!(parts.len() > 1);
        assert!(parts.iter().all(|part| part.len() <= CHUNK));
        assert_eq!(Session::parse_header(&parts.concat()), Some(session));
    }

    #[test]
    fn debug_output_never_shows_cookie_values() {
        let session = Session::from_cookies(signed_in_cookies()).unwrap();
        let shown = format!("{session:?}");
        assert!(shown.contains("SAPISID"));
        assert!(!shown.contains("abc/def"));
    }
}

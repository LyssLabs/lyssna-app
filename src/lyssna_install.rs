//! The install id that goes with the app's publishable key. api.lyssna.org
//! hands one out per copy of the app after a small proof of work; every
//! request that carries the key then also carries `Lyssna-Install`, so limits
//! and blocks apply to this copy and not to every copy of the app. The id is
//! kept in the credential store, registered on first use, and registered
//! again when the API stops accepting it.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::credential::Entry;

const SERVICE: &str = "Lyssna/Account";
const USER: &str = "install";
/// The API asks for about 18 bits; anything far above that is not ours to solve.
const MAX_DIFFICULTY: u32 = 28;
/// After a failed registration, wait this long before trying again.
const RETRY_AFTER: Duration = Duration::from_secs(30);

struct State {
    value: Option<String>,
    loaded: bool,
    retry_at: Option<Instant>,
}

static STATE: Mutex<State> = Mutex::new(State {
    value: None,
    loaded: false,
    retry_at: None,
});

#[derive(Deserialize)]
struct Challenge {
    challenge: String,
    difficulty: u32,
}

#[derive(Deserialize)]
struct Made {
    install: String,
}

/// The value for the `Lyssna-Install` header, registering a new install if
/// this copy has none. `None` when there is no key to register under or
/// registration failed; the API then answers `install_required` and the next
/// request tries again.
pub(crate) fn header(api: &str, key: &str) -> Option<String> {
    if key.is_empty() {
        return None;
    }
    let mut state = STATE.lock().ok()?;
    if !state.loaded {
        state.loaded = true;
        state.value = Entry::new(SERVICE, USER)
            .ok()
            .and_then(|entry| entry.get_password().ok())
            .filter(|value| !value.is_empty());
    }
    if state.value.is_some() {
        return state.value.clone();
    }
    if state.retry_at.is_some_and(|at| Instant::now() < at) {
        return None;
    }
    match register(api, key) {
        Ok(value) => {
            if let Ok(entry) = Entry::new(SERVICE, USER) {
                let _ = entry.set_password(&value);
            }
            state.value = Some(value.clone());
            state.retry_at = None;
            Some(value)
        }
        Err(_) => {
            state.retry_at = Some(Instant::now() + RETRY_AFTER);
            None
        }
    }
}

/// Drops the stored install after the API refused it, so the next request
/// registers a new one.
pub(crate) fn forget() {
    if let Ok(mut state) = STATE.lock() {
        state.value = None;
        state.loaded = true;
        state.retry_at = None;
    }
    if let Ok(entry) = Entry::new(SERVICE, USER) {
        let _ = entry.delete_credential();
    }
}

fn register(api: &str, key: &str) -> Result<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("Cannot start the install request")?;
    let challenge: Challenge = client
        .post(format!("{api}/v1/installs/challenge"))
        .header("Lyssna-Key", key)
        .json(&serde_json::json!({}))
        .send()
        .context("Cannot reach Lyssna")?
        .error_for_status()
        .context("Lyssna refused an install challenge")?
        .json()
        .context("Unreadable install challenge")?;
    ensure!(
        challenge.difficulty <= MAX_DIFFICULTY,
        "Lyssna asked for too much work"
    );
    let counter = solve(&challenge.challenge, challenge.difficulty);
    let made: Made = client
        .post(format!("{api}/v1/installs"))
        .header("Lyssna-Key", key)
        .json(&serde_json::json!({
            "challenge": challenge.challenge,
            "counter": counter.to_string(),
        }))
        .send()
        .context("Cannot reach Lyssna")?
        .error_for_status()
        .context("Lyssna refused to register this install")?
        .json()
        .context("Unreadable install")?;
    ensure!(!made.install.is_empty(), "Lyssna sent an empty install");
    Ok(made.install)
}

fn leading_zero_bits(hash: &[u8]) -> u32 {
    let mut bits = 0;
    for byte in hash {
        if *byte == 0 {
            bits += 8;
        } else {
            return bits + byte.leading_zeros();
        }
    }
    bits
}

/// The first counter for which sha256("<challenge>:<counter>") starts with
/// `difficulty` zero bits.
fn solve(challenge: &str, difficulty: u32) -> u64 {
    let mut counter = 0u64;
    loop {
        let hash = Sha256::digest(format!("{challenge}:{counter}").as_bytes());
        if leading_zero_bits(&hash) >= difficulty {
            return counter;
        }
        counter += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_leading_zero_bits() {
        assert_eq!(leading_zero_bits(&[0xff]), 0);
        assert_eq!(leading_zero_bits(&[0x0f]), 4);
        assert_eq!(leading_zero_bits(&[0x00, 0x01]), 15);
        assert_eq!(leading_zero_bits(&[0, 0, 0]), 24);
    }

    #[test]
    fn solution_has_the_asked_zero_bits() {
        let counter = solve("challenge-text", 12);
        let hash = Sha256::digest(format!("challenge-text:{counter}").as_bytes());
        assert!(leading_zero_bits(&hash) >= 12);
    }

    #[test]
    fn no_key_means_no_install() {
        assert!(header("http://127.0.0.1:1", "").is_none());
    }
}

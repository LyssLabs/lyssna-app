//! Lyssna's own community lyrics from api.lyssna.org: the served TTML, who
//! made it, and a play count when the lyrics show.

use std::fs;

use anyhow::{Context, Result, bail};
use reqwest::{StatusCode, blocking::Client};
use serde::Deserialize;

use crate::lyrics::{Credit, Lyrics, parse_ttml};
use crate::model::Track;

#[derive(Deserialize)]
struct LyricsView {
    song: Song,
    #[serde(default)]
    offset_ms: i64,
    #[serde(default)]
    contributors: Vec<Credit>,
    ttml: String,
}

#[derive(Deserialize)]
struct Song {
    id: String,
}

fn spotify_id(track: &Track) -> Option<&str> {
    track.id.as_deref().filter(|id| {
        !track.is_local && id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

pub fn fetch(client: &Client, track: &Track) -> Result<Lyrics> {
    let Some(id) = spotify_id(track) else {
        bail!("Lyssna has no lyrics for local songs");
    };
    let response = client
        .get(format!(
            "{}/v1/lyrics/spotify/{id}",
            crate::lyssna_account::API
        ))
        .send()
        .context("Cannot reach Lyssna")?;
    if response.status() == StatusCode::NOT_FOUND {
        bail!("No Lyssna lyrics for this song");
    }
    if !response.status().is_success() {
        bail!("Lyssna lyrics failed: HTTP {}", response.status().as_u16());
    }
    let view: LyricsView = response.json().context("Unreadable Lyssna lyrics")?;
    let mut lyrics = parse_ttml(&view.ttml, track.duration_ms)?;
    shift(&mut lyrics, view.offset_ms);
    lyrics.source = "lyssna".into();
    lyrics.credits = view.contributors;
    lyrics.song_id = Some(view.song.id);
    Ok(lyrics)
}

/// The song's offset for this platform moves every timestamp.
fn shift(lyrics: &mut Lyrics, offset_ms: i64) {
    let moved = |time: &mut u64| *time = time.saturating_add_signed(offset_ms);
    for line in &mut lyrics.lines {
        moved(&mut line.start_ms);
        moved(&mut line.end_ms);
        for word in &mut line.words {
            moved(&mut word.start_ms);
            moved(&mut word.end_ms);
        }
    }
}

/// A random ID kept on this computer, so plays count once per listener
/// without signing in.
fn client_id() -> Option<String> {
    let file = crate::config::data_dir().ok()?.join("lyssna-client-id");
    if let Ok(id) = fs::read_to_string(&file)
        && id.trim().len() >= 16
    {
        return Some(id.trim().to_owned());
    }
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).ok()?;
    let id: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let _ = fs::write(&file, &id);
    Some(id)
}

/// Counts a play while Lyssna's lyrics show; the server keeps one per
/// listener per song every 30 minutes.
pub fn record_play(client: &Client, song_id: &str) {
    if !song_id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return;
    }
    let mut request = client.post(format!(
        "{}/v1/songs/{song_id}/play",
        crate::lyssna_account::API
    ));
    if let Some(id) = client_id() {
        request = request.json(&serde_json::json!({ "client_id": id }));
    }
    let _ = request.send();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_lyrics_view_and_applies_the_offset() {
        let view: LyricsView = serde_json::from_value(serde_json::json!({
            "song": {"id": "0193", "title": "T", "artists": ["A"]},
            "channel": "curated", "status": "approved", "revision_id": "r", "timing_kind": "word",
            "offset_ms": -250, "score": 0, "vote_count": 0,
            "contributors": [
                {"user_id": "u1", "username": "Arashii", "avatar_url": "https://api.lyssna.org/v1/images/a.png",
                 "units": 10, "share": 75.0, "text_share": 100.0, "timing_share": 50.0, "original": true},
                {"units": 3, "share": 25.0, "text_share": 0.0, "timing_share": 50.0, "original": false}
            ],
            "ttml": r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><div><p begin="1.000" end="2.000"><span begin="1.000" end="2.000">Hi</span></p></div></body></tt>"#
        }))
        .unwrap();
        assert_eq!(view.contributors[0].username.as_deref(), Some("Arashii"));
        assert!(view.contributors[1].username.is_none());
        let mut lyrics = parse_ttml(&view.ttml, 3000).unwrap();
        shift(&mut lyrics, view.offset_ms);
        assert_eq!(lyrics.lines[0].start_ms, 750);
        assert_eq!(lyrics.lines[0].words[0].end_ms, 1750);
        shift(&mut lyrics, -5000);
        assert_eq!(lyrics.lines[0].start_ms, 0);
    }

    #[test]
    fn only_spotify_tracks_are_looked_up() {
        let mut track = Track {
            id: Some("4cOdK2wGLETKBW3PvgPWqT".into()),
            ..Track::default()
        };
        assert!(spotify_id(&track).is_some());
        track.is_local = true;
        assert!(spotify_id(&track).is_none());
        track.is_local = false;
        track.id = Some("../etc".into());
        assert!(spotify_id(&track).is_none());
    }
}

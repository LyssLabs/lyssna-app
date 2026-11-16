//! YouTube Music's web client API (InnerTube), as ytmusicapi calls it.

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::time::Duration;

use crate::model::{Album, Artist, Artwork, Track};

const BASE: &str = "https://music.youtube.com/youtubei/v1";
const CLIENT_NAME: &str = "WEB_REMIX";
const CLIENT_VERSION: &str = "1.20251110.01.00";
/// The search filter YouTube Music's Songs chip sends.
const SONGS_FILTER: &str = "EgWKAQIIAWoMEA4QChADEAQQCRAF";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36";

pub const URI_PREFIX: &str = "ytmusic:";

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()?)
}

fn call(endpoint: &str, body: Value) -> Result<Value> {
    let mut body = body;
    body["context"] = json!({
        "client": {"clientName": CLIENT_NAME, "clientVersion": CLIENT_VERSION, "hl": "en"}
    });
    let response = client()?
        .post(format!("{BASE}/{endpoint}?prettyPrint=false"))
        .header("Origin", "https://music.youtube.com")
        .header("X-Goog-Api-Format-Version", "1")
        .json(&body)
        .send()
        .context("Cannot reach YouTube Music")?;
    ensure!(
        response.status().is_success(),
        "YouTube Music returned HTTP {}",
        response.status().as_u16()
    );
    response
        .json()
        .context("YouTube Music sent an unreadable reply")
}

pub fn search_songs(query: &str) -> Result<Vec<Track>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let reply = call("search", json!({"query": query, "params": SONGS_FILTER}))?;
    Ok(songs_from_search(&reply))
}

fn runs(column: &Value) -> &[Value] {
    column["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"]
        .as_array()
        .map_or(&[], Vec::as_slice)
}

fn page_type(run: &Value) -> &str {
    run["navigationEndpoint"]["browseEndpoint"]["browseEndpointContextSupportedConfigs"]
        ["browseEndpointContextMusicConfig"]["pageType"]
        .as_str()
        .unwrap_or_default()
}

fn browse_id(run: &Value) -> Option<&str> {
    run["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str()
}

fn duration_ms(text: &str) -> Option<u64> {
    let mut total = 0u64;
    let mut parts = 0;
    for part in text.split(':') {
        total = total * 60 + part.parse::<u64>().ok()?;
        parts += 1;
    }
    (2..=3).contains(&parts).then_some(total * 1000)
}

/// YouTube's thumbnail links carry their size; ask for one big enough for
/// the Now playing artwork.
fn artwork(item: &Value) -> Vec<Artwork> {
    let thumbnails = item["thumbnail"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let Some(url) = thumbnails.last().and_then(|thumb| thumb["url"].as_str()) else {
        return Vec::new();
    };
    match url.rsplit_once("=w") {
        Some((base, _)) if url.contains("googleusercontent.com") => vec![
            Artwork {
                url: format!("{base}=w544-h544-l90-rj"),
                width: Some(544),
            },
            Artwork {
                url: format!("{base}=w120-h120-l90-rj"),
                width: Some(120),
            },
        ],
        _ => vec![Artwork {
            url: url.to_owned(),
            width: None,
        }],
    }
}

fn song(item: &Value) -> Option<Track> {
    let columns = item["flexColumns"].as_array()?;
    let title = runs(columns.first()?).first()?;
    let video_id = title["navigationEndpoint"]["watchEndpoint"]["videoId"]
        .as_str()
        .or_else(|| item["playlistItemData"]["videoId"].as_str())
        .filter(|id| crate::ytm_tools::valid_video_id(id))?;
    let details = columns.get(1).map_or(&[][..], runs);
    let groups: Vec<&[Value]> = details
        .split(|run| run["text"].as_str() == Some(" • "))
        .collect();
    let artists = groups
        .first()
        .map_or(&[][..], |group| group)
        .iter()
        .filter(|run| !matches!(run["text"].as_str(), Some(" & " | ", " | " and ")))
        .filter(|run| matches!(page_type(run), "MUSIC_PAGE_TYPE_ARTIST" | ""))
        .filter_map(|run| {
            let name = run["text"].as_str()?.to_owned();
            let uri =
                browse_id(run).map_or_else(String::new, |id| format!("{URI_PREFIX}artist:{id}"));
            Some(Artist {
                name,
                uri,
                ..Artist::default()
            })
        })
        .collect();
    let album_run = details
        .iter()
        .find(|run| page_type(run) == "MUSIC_PAGE_TYPE_ALBUM");
    let duration = groups
        .last()
        .and_then(|group| group.first())
        .and_then(|run| run["text"].as_str())
        .and_then(duration_ms)
        .unwrap_or_default();
    let explicit = item["badges"].as_array().is_some_and(|badges| {
        badges.iter().any(|badge| {
            badge["musicInlineBadgeRenderer"]["icon"]["iconType"].as_str()
                == Some("MUSIC_EXPLICIT_BADGE")
        })
    });
    Some(Track {
        id: None,
        uri: format!("{URI_PREFIX}track:{video_id}"),
        name: title["text"].as_str()?.to_owned(),
        artists,
        album: Album {
            name: album_run
                .and_then(|run| run["text"].as_str())
                .unwrap_or_default()
                .to_owned(),
            uri: album_run
                .and_then(browse_id)
                .map_or_else(String::new, |id| format!("{URI_PREFIX}album:{id}")),
            images: artwork(item),
            ..Album::default()
        },
        duration_ms: duration,
        explicit,
        ..Track::default()
    })
}

fn songs_from_search(reply: &Value) -> Vec<Track> {
    let sections = reply["contents"]["tabbedSearchResultsRenderer"]["tabs"][0]["tabRenderer"]
        ["content"]["sectionListRenderer"]["contents"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    sections
        .iter()
        .filter_map(|section| section["musicShelfRenderer"]["contents"].as_array())
        .flatten()
        .filter_map(|entry| song(&entry["musicResponsiveListItemRenderer"]))
        .collect()
}

/// The video id in a `ytmusic:track:<id>` URI.
pub fn video_id(uri: &str) -> Option<&str> {
    uri.strip_prefix(URI_PREFIX)?
        .strip_prefix("track:")
        .filter(|id| crate::ytm_tools::valid_video_id(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/ytmusic-search-songs.json")).unwrap()
    }

    #[test]
    fn reads_songs_from_a_search_reply() {
        let songs = songs_from_search(&fixture());
        assert_eq!(songs.len(), 3);
        let first = &songs[0];
        assert_eq!(first.uri, "ytmusic:track:wU26xVT_vBU");
        assert_eq!(first.name, "One More Time (Radio Edit)");
        assert_eq!(first.artists.len(), 1);
        assert_eq!(first.artists[0].name, "Daft Punk");
        assert_eq!(
            first.artists[0].uri,
            "ytmusic:artist:UCRr1xG_2WIDs18a6cIiCxeA"
        );
        assert_eq!(first.album.name, "One More Time");
        assert_eq!(first.album.uri, "ytmusic:album:MPREb_uWQeJO6Z53u");
        assert_eq!(first.duration_ms, 321_000);
        assert_eq!(first.album.images[0].width, Some(544));
        assert!(first.album.images[0].url.ends_with("=w544-h544-l90-rj"));
        assert!(first.id.is_none());
    }

    #[test]
    fn keeps_every_artist_of_a_shared_song() {
        let songs = songs_from_search(&fixture());
        let names: Vec<_> = songs[2].artists.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["DJ KVNXD", "itssimannn!"]);
        assert_eq!(songs[2].duration_ms, 103_000);
    }

    #[test]
    fn found_songs_come_back_from_the_page_intact() {
        for song in songs_from_search(&fixture()) {
            let sent = serde_json::to_string(&song).unwrap();
            let back: Track = serde_json::from_str(&sent).unwrap();
            assert_eq!(back.uri, song.uri);
        }
    }

    #[test]
    fn reads_clock_durations() {
        assert_eq!(duration_ms("5:21"), Some(321_000));
        assert_eq!(duration_ms("1:02:03"), Some(3_723_000));
        assert_eq!(duration_ms("859M plays"), None);
        assert_eq!(duration_ms("21"), None);
    }

    #[test]
    fn only_youtube_music_song_uris_give_a_video_id() {
        assert_eq!(video_id("ytmusic:track:wU26xVT_vBU"), Some("wU26xVT_vBU"));
        assert_eq!(video_id("spotify:track:wU26xVT_vBU"), None);
        assert_eq!(video_id("ytmusic:track:--exec=calc"), None);
    }
}

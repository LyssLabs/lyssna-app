//! Matching lyric files loaded in bulk to their songs, and writing the
//! matched Spotify ID back into a TTML file so it finds its song again.

/// AMLL's TTML namespace, which its lyric database files carry their
/// `spotifyId`, `isrc`, `musicName` and `artists` metadata in.
const AMLL: &str = "http://www.example.com/ns/amll";

/// What a lyric file says about the song it belongs to.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SongHint {
    pub spotify_id: Option<String>,
    pub isrc: Option<String>,
    /// Words to search for the song by, from its metadata or file name.
    pub query: String,
}

fn spotify_id(value: &str) -> Option<String> {
    let id = value.trim().rsplit(':').next()?;
    (id.len() == 22 && id.bytes().all(|byte| byte.is_ascii_alphanumeric())).then(|| id.into())
}

/// A token in a file name that looks like a Spotify ID: 22 letters and
/// digits, with both, so ordinary long words don't count.
fn id_in_name(stem: &str) -> Option<String> {
    stem.split(|c: char| !c.is_ascii_alphanumeric())
        .find(|token| {
            token.len() == 22
                && token.bytes().any(|byte| byte.is_ascii_digit())
                && token.bytes().any(|byte| byte.is_ascii_alphabetic())
        })
        .map(str::to_owned)
}

pub fn hint(filename: &str, text: &str) -> SongHint {
    let stem = std::path::Path::new(filename)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let mut hint = SongHint::default();
    let (mut title, mut artists) = (None, Vec::new());
    if let Ok(document) = roxmltree::Document::parse(text) {
        for meta in document
            .descendants()
            .filter(|node| node.tag_name().name() == "meta")
        {
            let (Some(key), Some(value)) = (meta.attribute("key"), meta.attribute("value")) else {
                continue;
            };
            let value = value.trim();
            match key {
                "spotifyId" if hint.spotify_id.is_none() => hint.spotify_id = spotify_id(value),
                "isrc" if hint.isrc.is_none() && !value.is_empty() => {
                    hint.isrc = Some(value.to_ascii_uppercase())
                }
                "musicName" if title.is_none() && !value.is_empty() => {
                    title = Some(value.to_owned())
                }
                "artists" if !value.is_empty() => artists.push(value.to_owned()),
                _ => {}
            }
        }
    }
    let named = id_in_name(stem);
    if hint.spotify_id.is_none() {
        hint.spotify_id = named.clone();
    }
    hint.query = match title {
        Some(title) => [title, artists.join(" ")].join(" "),
        // Lyssna saves lyrics as "Song - Artists".
        None => stem
            .replace(named.as_deref().unwrap_or("\u{0}"), " ")
            .replace(" - ", " ")
            .replace(['_', '.'], " "),
    }
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ");
    hint
}

/// The TTML file with `id` recorded as its song's Spotify ID, as AMLL's
/// files carry it. Unchanged if it already names one, or isn't TTML this
/// can safely add to.
pub fn with_spotify_id(text: &str, id: &str) -> String {
    if spotify_id(id).is_none()
        || roxmltree::Document::parse(text).is_err()
        || hint("", text).spotify_id.is_some()
    {
        return text.to_owned();
    }
    let meta = format!(r#"<amll:meta key="spotifyId" value="{id}"/>"#);
    let open = |name: &str| {
        text.match_indices(&format!("<{name}"))
            .map(|(at, _)| at)
            .find(|&at| {
                text[at + name.len() + 1..]
                    .starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/')
            })
    };
    let tag_end = |at: usize| text[at..].find('>').map(|end| at + end);
    let mut out = text.to_owned();
    if let Some(at) = text.find("</metadata>") {
        out.insert_str(at, &meta);
    } else if let Some(at) = open("metadata")
        && let Some(end) = tag_end(at)
    {
        if text[..end].ends_with('/') {
            out.replace_range(at..=end, &format!("<metadata>{meta}</metadata>"));
        } else {
            out.insert_str(end + 1, &meta);
        }
    } else if let Some(at) = open("head")
        && let Some(end) = tag_end(at)
    {
        if text[..end].ends_with('/') {
            out.replace_range(
                at..=end,
                &format!("<head><metadata>{meta}</metadata></head>"),
            );
        } else {
            out.insert_str(end + 1, &format!("<metadata>{meta}</metadata>"));
        }
    } else if let Some(at) = open("tt")
        && let Some(end) = tag_end(at)
        && !text[..end].ends_with('/')
    {
        out.insert_str(
            end + 1,
            &format!("<head><metadata>{meta}</metadata></head>"),
        );
    } else {
        return text.to_owned();
    }
    if !text.contains("xmlns:amll=")
        && let Some(at) = open("tt")
    {
        out.insert_str(at + 3, &format!(r#" xmlns:amll="{AMLL}""#));
    }
    match roxmltree::Document::parse(&out) {
        Ok(_) if hint("", &out).spotify_id.as_deref() == Some(id) => out,
        _ => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "4uLU6hMCjMI75M1A2tKUQC";
    const BODY: &str = r#"<body><div><p begin="0s" end="1s">Hi</p></div></body>"#;

    #[test]
    fn amll_metadata_names_the_song() {
        let text = format!(
            r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:amll="{AMLL}"><head><metadata>
            <amll:meta key="musicName" value="Never Gonna Give You Up"/>
            <amll:meta key="artists" value="Rick Astley"/>
            <amll:meta key="isrc" value="gbarl9300135"/>
            <amll:meta key="spotifyId" value="{ID}"/>
            </metadata></head>{BODY}</tt>"#
        );
        assert_eq!(
            hint("whatever.ttml", &text),
            SongHint {
                spotify_id: Some(ID.into()),
                isrc: Some("GBARL9300135".into()),
                query: "Never Gonna Give You Up Rick Astley".into(),
            }
        );
    }

    #[test]
    fn file_names_give_an_id_or_words_to_search() {
        let plain = r#"<tt xmlns="http://www.w3.org/ns/ttml"></tt>"#;
        assert_eq!(
            hint(&format!("spotify_track_{ID}.ttml"), plain)
                .spotify_id
                .as_deref(),
            Some(ID)
        );
        let named = hint("Sugar - System Of A Down.ttml", plain);
        assert_eq!(named.spotify_id, None);
        assert_eq!(named.query, "Sugar System Of A Down");
        assert_eq!(
            hint("Supercalifragilisticexp.lrc", "[00:01.00]Hi").spotify_id,
            None
        );
    }

    #[test]
    fn a_matched_id_is_written_into_the_file() {
        for text in [
            format!(r#"<tt xmlns="http://www.w3.org/ns/ttml">{BODY}</tt>"#),
            format!(r#"<tt xmlns="http://www.w3.org/ns/ttml"><head/>{BODY}</tt>"#),
            format!(
                r#"<tt xmlns="http://www.w3.org/ns/ttml"><head><metadata><ttm:agent xmlns:ttm="http://www.w3.org/ns/ttml#metadata" type="person" xml:id="v1"/></metadata></head>{BODY}</tt>"#
            ),
            format!(
                r#"<?xml version="1.0"?><tt xmlns="http://www.w3.org/ns/ttml"><head><metadata/></head>{BODY}</tt>"#
            ),
        ] {
            let tagged = with_spotify_id(&text, ID);
            assert_eq!(
                hint("x.ttml", &tagged).spotify_id.as_deref(),
                Some(ID),
                "{tagged}"
            );
            assert!(
                crate::lyrics::parse_ttml(&tagged, 10_000).is_ok(),
                "{tagged}"
            );
        }
        let other = "1111111111111111111111";
        let named = with_spotify_id(
            &format!(
                r#"<tt xmlns:amll="{AMLL}"><head><metadata><amll:meta key="spotifyId" value="{other}"/></metadata></head></tt>"#
            ),
            ID,
        );
        assert!(named.contains(other) && !named.contains(ID));
        assert_eq!(with_spotify_id("[00:01.00]Hi", ID), "[00:01.00]Hi");
    }
}

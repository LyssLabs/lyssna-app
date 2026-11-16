//! yt-dlp and Deno, fetched on the listener's machine from their official
//! GitHub releases, never bundled. yt-dlp turns a YouTube Music song into an
//! audio stream link; Deno runs the JavaScript checks YouTube asks for.

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::soloist::HiddenCommand;

const YT_DLP_REPO: &str = "yt-dlp/yt-dlp";
const DENO_REPO: &str = "denoland/deno";
const YT_DLP_EXE: &str = if cfg!(windows) {
    "yt-dlp.exe"
} else {
    "yt-dlp"
};
const DENO_EXE: &str = if cfg!(windows) { "deno.exe" } else { "deno" };
const YT_DLP_ASSET: &str = if cfg!(windows) {
    "yt-dlp.exe"
} else if cfg!(target_os = "macos") {
    "yt-dlp_macos"
} else {
    "yt-dlp_linux"
};
const DENO_ASSET: &str = if cfg!(windows) {
    "deno-x86_64-pc-windows-msvc.zip"
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    "deno-aarch64-apple-darwin.zip"
} else if cfg!(target_os = "macos") {
    "deno-x86_64-apple-darwin.zip"
} else {
    "deno-x86_64-unknown-linux-gnu.zip"
};
const MAX_DOWNLOAD: u64 = 256 * 1024 * 1024;
const UPDATE_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// One update or resolve at a time, so a stream lookup never runs a program
/// that is being replaced.
static TOOLS: Mutex<()> = Mutex::new(());

#[derive(Default, Serialize, Deserialize)]
struct Installed {
    yt_dlp: String,
    deno: String,
    checked_at: u64,
}

pub struct Tools {
    pub yt_dlp: PathBuf,
    pub deno: PathBuf,
}

fn bin_dir() -> Result<PathBuf> {
    Ok(crate::config::data_dir()?.join("ytmusic").join("bin"))
}

fn installed_path(bin: &Path) -> PathBuf {
    bin.join("installed.json")
}

fn read_installed(bin: &Path) -> Installed {
    fs::read(installed_path(bin))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn client(redirects: bool) -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(concat!("Lyssna/", env!("CARGO_PKG_VERSION")))
        .redirect(if redirects {
            reqwest::redirect::Policy::limited(5)
        } else {
            reqwest::redirect::Policy::none()
        })
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .build()?)
}

/// The tag GitHub's `releases/latest` redirects to.
fn tag_from_location(location: &str) -> Option<String> {
    let tag = location.rsplit_once("/releases/tag/")?.1;
    let valid = !tag.is_empty()
        && tag.len() <= 64
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    valid.then(|| tag.to_owned())
}

fn latest_tag(repo: &str) -> Result<String> {
    let response = client(false)?
        .get(format!("https://github.com/{repo}/releases/latest"))
        .send()
        .with_context(|| format!("Cannot reach GitHub for {repo}"))?;
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    tag_from_location(location).with_context(|| format!("GitHub gave no latest release for {repo}"))
}

fn download(url: &str) -> Result<Vec<u8>> {
    let response = client(true)?
        .get(url)
        .send()
        .with_context(|| format!("Cannot download {url}"))?;
    ensure!(
        response.status().is_success(),
        "{url} returned HTTP {}",
        response.status().as_u16()
    );
    let mut bytes = Vec::new();
    response.take(MAX_DOWNLOAD + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_DOWNLOAD, "{url} is too large");
    Ok(bytes)
}

/// The SHA-256 published for `asset`: yt-dlp and Deno's Mac and Linux
/// files list `<hash>  <name>` lines; Deno's Windows file is PowerShell's
/// `Hash : <HASH>`.
fn published_hash(sums: &str, asset: &str) -> Option<String> {
    let hex = |word: &str| word.len() == 64 && word.chars().all(|c| c.is_ascii_hexdigit());
    sums.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        let first = words.next()?;
        let hash = if first == "Hash" {
            words.nth(1)?
        } else if words.next()?.trim_start_matches('*') == asset {
            first
        } else {
            return None;
        };
        hex(hash).then(|| hash.to_ascii_lowercase())
    })
}

fn verified(bytes: &[u8], expected: &str, asset: &str) -> Result<()> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    ensure!(
        actual == expected,
        "{asset} does not match its published checksum"
    );
    Ok(())
}

fn zip_member(archive: Vec<u8>, wanted: &str) -> Result<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))?;
    let entry = zip
        .by_name(wanted)
        .with_context(|| format!("Download has no {wanted}"))?;
    let mut bytes = Vec::new();
    entry.take(MAX_DOWNLOAD + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_DOWNLOAD, "{wanted} is too large");
    Ok(bytes)
}

/// Writes `bytes` beside `exe`, checks the program starts, then replaces it.
fn install(bin: &Path, exe: &str, bytes: &[u8], version_starts: &[u8]) -> Result<()> {
    fs::create_dir_all(bin)?;
    let partial = bin.join(format!("{exe}.download"));
    let mut file = fs::File::create(&partial)?;
    std::io::Write::write_all(&mut file, bytes)?;
    file.sync_all()?;
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&partial, fs::Permissions::from_mode(0o755))?;
    }
    let started = Command::new(&partial)
        .arg("--version")
        .creation_flags_hidden()
        .output()
        .map_err(anyhow::Error::from)
        .and_then(|output| {
            ensure!(
                output.status.success() && output.stdout.starts_with(version_starts),
                "Downloaded {exe} did not start"
            );
            Ok(())
        });
    if let Err(error) = started {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    fs::rename(&partial, bin.join(exe))?;
    Ok(())
}

fn fetch_yt_dlp(bin: &Path, tag: &str) -> Result<()> {
    let base = format!("https://github.com/{YT_DLP_REPO}/releases/download/{tag}");
    let sums = String::from_utf8(download(&format!("{base}/SHA2-256SUMS"))?)?;
    let expected =
        published_hash(&sums, YT_DLP_ASSET).context("yt-dlp has no published checksum")?;
    let bytes = download(&format!("{base}/{YT_DLP_ASSET}"))?;
    verified(&bytes, &expected, YT_DLP_ASSET)?;
    install(bin, YT_DLP_EXE, &bytes, tag.as_bytes())
}

fn fetch_deno(bin: &Path, tag: &str) -> Result<()> {
    let base = format!("https://github.com/{DENO_REPO}/releases/download/{tag}");
    let sums = String::from_utf8(download(&format!("{base}/{DENO_ASSET}.sha256sum"))?)?;
    let expected = published_hash(&sums, DENO_ASSET).context("Deno has no published checksum")?;
    let archive = download(&format!("{base}/{DENO_ASSET}"))?;
    verified(&archive, &expected, DENO_ASSET)?;
    install(bin, DENO_EXE, &zip_member(archive, DENO_EXE)?, b"deno ")
}

/// The installed helpers, fetched or updated first when due.
pub fn ensure() -> Result<Tools> {
    let _guard = TOOLS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let bin = bin_dir()?;
    let mut installed = read_installed(&bin);
    let present = bin.join(YT_DLP_EXE).is_file() && bin.join(DENO_EXE).is_file();
    if !present || now().saturating_sub(installed.checked_at) >= UPDATE_EVERY.as_secs() {
        let mut update = || -> Result<()> {
            let tag = latest_tag(YT_DLP_REPO)?;
            if tag != installed.yt_dlp || !bin.join(YT_DLP_EXE).is_file() {
                fetch_yt_dlp(&bin, &tag)?;
                installed.yt_dlp = tag;
            }
            let tag = latest_tag(DENO_REPO)?;
            if tag != installed.deno || !bin.join(DENO_EXE).is_file() {
                fetch_deno(&bin, &tag)?;
                installed.deno = tag;
            }
            Ok(())
        };
        match update() {
            Ok(()) => {
                installed.checked_at = now();
                fs::write(installed_path(&bin), serde_json::to_vec(&installed)?)?;
            }
            // An update that fails keeps the working copies.
            Err(error) if present => {
                crate::native::diagnostic(&format!("YouTube Music helpers not updated: {error:#}"))
            }
            Err(error) => return Err(error),
        }
    }
    Ok(Tools {
        yt_dlp: bin.join(YT_DLP_EXE),
        deno: bin.join(DENO_EXE),
    })
}

/// YouTube video ids are 11 characters of base64url.
pub fn valid_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

#[derive(Debug, PartialEq)]
pub struct Stream {
    pub url: String,
    pub size: Option<u64>,
    pub duration_ms: Option<u64>,
    /// yt-dlp's details for the song, which its download reuses.
    pub info: String,
}

/// The audio link in yt-dlp's `--dump-json` output for one chosen format.
fn stream_from_info(info_text: &str) -> Result<Stream> {
    let info: serde_json::Value =
        serde_json::from_str(info_text).context("yt-dlp gave no song details")?;
    let chosen = info
        .get("requested_formats")
        .and_then(|formats| formats.get(0))
        .unwrap_or(&info);
    let url = chosen
        .get("url")
        .and_then(|url| url.as_str())
        .filter(|url| url.starts_with("https://"))
        .context("yt-dlp gave no audio link")?;
    let size = chosen
        .get("filesize")
        .or_else(|| chosen.get("filesize_approx"))
        .and_then(serde_json::Value::as_u64);
    let duration_ms = info
        .get("duration")
        .and_then(serde_json::Value::as_f64)
        .map(|seconds| (seconds * 1000.0).round() as u64);
    Ok(Stream {
        url: url.to_owned(),
        size,
        duration_ms,
        info: info_text.to_owned(),
    })
}

/// The AAC stream for a song: Lyssna's decoder plays AAC, not Opus.
pub fn resolve(video_id: &str) -> Result<Stream> {
    if !valid_video_id(video_id) {
        bail!("Not a YouTube Music song");
    }
    let tools = ensure()?;
    let _guard = TOOLS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let output = Command::new(&tools.yt_dlp)
        .arg("--no-config")
        .arg("--no-warnings")
        .arg("--no-playlist")
        .arg("--js-runtimes")
        .arg(format!("deno:{}", tools.deno.display()))
        .args(["-f", "140/bestaudio[acodec^=mp4a]", "--dump-json", "--"])
        .arg(format!("https://music.youtube.com/watch?v={video_id}"))
        .creation_flags_hidden()
        .output()
        .context("Cannot run yt-dlp")?;
    if !output.status.success() {
        let reason = String::from_utf8_lossy(&output.stderr);
        let reason = reason
            .lines()
            .rev()
            .find(|line| line.contains("ERROR"))
            .unwrap_or("no reason given");
        bail!(
            "yt-dlp could not open the song: {}",
            reason.chars().take(200).collect::<String>()
        );
    }
    stream_from_info(&String::from_utf8_lossy(&output.stdout))
}

/// yt-dlp downloading the song it described in `info_file` to its stdout.
/// It knows the headers and tokens YouTube checks on the audio link, so a
/// plain request for the link is refused with HTTP 403.
pub enum Song<'a> {
    /// The details a lookup already wrote, so YouTube is not asked twice.
    Described(&'a Path),
    /// A fresh lookup, for when YouTube refused the described link.
    Fresh(&'a str),
}

pub fn download_song(song: &Song) -> Result<std::process::Child> {
    let tools = ensure()?;
    let mut command = Command::new(&tools.yt_dlp);
    match song {
        Song::Described(info_file) => {
            command.arg("--load-info-json").arg(info_file);
        }
        Song::Fresh(video_id) => {
            ensure!(valid_video_id(video_id), "Not a YouTube Music song");
            command
                .arg("--no-playlist")
                .arg(format!("https://music.youtube.com/watch?v={video_id}"));
        }
    }
    command
        .arg("--no-config")
        .arg("--quiet")
        .arg("--no-warnings")
        .arg("--no-progress")
        .arg("--no-part")
        .arg("--js-runtimes")
        .arg(format!("deno:{}", tools.deno.display()))
        .args(["-f", "140/bestaudio[acodec^=mp4a]", "-o", "-"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .creation_flags_hidden()
        .spawn()
        .context("Cannot run yt-dlp")
}

/// The last error line yt-dlp printed.
pub fn failure(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    text.lines()
        .rev()
        .find(|line| line.contains("ERROR"))
        .unwrap_or("no reason given")
        .chars()
        .take(200)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_tag_github_redirects_to() {
        assert_eq!(
            tag_from_location("https://github.com/yt-dlp/yt-dlp/releases/tag/2026.10.26")
                .as_deref(),
            Some("2026.10.26")
        );
        assert_eq!(
            tag_from_location("https://github.com/denoland/deno/releases/tag/v2.9.8").as_deref(),
            Some("v2.9.8")
        );
        assert_eq!(
            tag_from_location("https://github.com/denoland/deno/releases"),
            None
        );
        assert_eq!(
            tag_from_location("https://github.com/x/y/releases/tag/../../evil"),
            None
        );
    }

    #[test]
    fn finds_published_checksums_in_both_formats() {
        let hash = "f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d";
        let sums = format!("{}  yt-dlp\n{hash}  yt-dlp.exe\n", "a".repeat(64));
        assert_eq!(published_hash(&sums, "yt-dlp.exe").as_deref(), Some(hash));
        assert_eq!(published_hash(&sums, "yt-dlp_macos"), None);
        let powershell = "\nAlgorithm : SHA256\nHash      : CBA84B26E0DF5AE7F74FC66B86A95B866872A86292EBD84FA1E959927EDD93BF\nPath      : C:\\a\\deno\\deno\\target\\release\\deno-x86_64-pc-windows-msvc.zip\n";
        assert_eq!(
            published_hash(powershell, "deno-x86_64-pc-windows-msvc.zip").as_deref(),
            Some("cba84b26e0df5ae7f74fc66b86a95b866872a86292ebd84fa1e959927edd93bf")
        );
        let single = format!("{hash}  deno-aarch64-apple-darwin.zip\n");
        assert_eq!(
            published_hash(&single, "deno-aarch64-apple-darwin.zip").as_deref(),
            Some(hash)
        );
    }

    #[test]
    fn rejects_downloads_that_do_not_match() {
        let expected = format!("{:x}", Sha256::digest(b"yt-dlp"));
        assert!(verified(b"yt-dlp", &expected, "yt-dlp.exe").is_ok());
        assert!(verified(b"tampered", &expected, "yt-dlp.exe").is_err());
    }

    #[test]
    fn only_youtube_ids_reach_yt_dlp() {
        assert!(valid_video_id("wU26xVT_vBU"));
        assert!(valid_video_id("-abcdefghij"));
        assert!(!valid_video_id("--exec=calc"));
        assert!(!valid_video_id("wU26xVT_vB"));
        assert!(!valid_video_id("wU26xVT vBU"));
    }

    #[test]
    fn reads_the_audio_link_from_song_details() {
        let info = r#"{"id":"wU26xVT_vBU","duration":320.5,"url":"https://rr1.googlevideo.com/videoplayback?itag=140","filesize":5181590}"#;
        assert_eq!(
            stream_from_info(info).unwrap(),
            Stream {
                url: "https://rr1.googlevideo.com/videoplayback?itag=140".into(),
                size: Some(5_181_590),
                duration_ms: Some(320_500),
                info: info.into(),
            }
        );
        assert!(stream_from_info(r#"{"url":"http://insecure.example/a"}"#).is_err());
        assert!(stream_from_info("not json").is_err());
    }
}

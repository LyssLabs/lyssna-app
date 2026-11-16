//! Plays YouTube Music songs on this computer. Spotify's engine needs a
//! Spotify session, so YouTube Music has its own: the song's AAC stream is
//! fetched into a cache, then decoded by symphonia through rodio.

use anyhow::{Context, Result, ensure};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::model::Track;

const CHUNK: u64 = 10 * 1024 * 1024;
const MAX_SONG: u64 = 200 * 1024 * 1024;
const TICK: Duration = Duration::from_millis(250);

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// A YouTube Music song is this computer's current playback, so Spotify's
/// reports must not replace it.
pub fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

#[derive(Clone, Default)]
pub struct Status {
    pub track: Option<Track>,
    pub playing: bool,
    pub position_ms: u64,
    pub volume: u8,
}

enum Command {
    Play(PathBuf, Box<Track>),
    Pause,
    Resume,
    Seek(u64),
    Volume(u8),
    Stop,
}

pub struct Player {
    commands: Mutex<Option<mpsc::Sender<Command>>>,
    status: Arc<Mutex<Status>>,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            commands: Mutex::new(None),
            status: Arc::new(Mutex::new(Status {
                volume: 100,
                ..Status::default()
            })),
        }
    }
}

impl Player {
    pub fn status(&self) -> Status {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_default()
    }

    fn send(&self, command: Command) -> Result<()> {
        let mut commands = self
            .commands
            .lock()
            .map_err(|_| anyhow::anyhow!("YouTube Music player unavailable"))?;
        if commands.is_none() {
            let (sender, receiver) = mpsc::channel();
            let status = self.status.clone();
            std::thread::Builder::new()
                .name("ytmusic-player".into())
                .spawn(move || run(&receiver, &status))?;
            *commands = Some(sender);
        }
        if let Some(sender) = commands.as_ref()
            && sender.send(command).is_ok()
        {
            return Ok(());
        }
        *commands = None;
        anyhow::bail!("YouTube Music player stopped")
    }

    pub fn play(&self, path: PathBuf, track: Track) -> Result<()> {
        ACTIVE.store(true, Ordering::Release);
        self.send(Command::Play(path, Box::new(track)))
    }
    pub fn pause(&self) -> Result<()> {
        self.send(Command::Pause)
    }
    pub fn resume(&self) -> Result<()> {
        self.send(Command::Resume)
    }
    pub fn seek(&self, position_ms: u64) -> Result<()> {
        self.send(Command::Seek(position_ms))
    }
    pub fn volume(&self, percent: u8) -> Result<()> {
        self.send(Command::Volume(percent.min(100)))
    }
    /// Hands this computer back to Spotify.
    pub fn stop(&self) {
        ACTIVE.store(false, Ordering::Release);
        let _ = self.send(Command::Stop);
    }
}

/// Perceived loudness follows the square of the slider, as Spotify's does.
fn gain(percent: u8) -> f32 {
    (f32::from(percent) / 100.0).powi(2)
}

fn run(commands: &mpsc::Receiver<Command>, status: &Mutex<Status>) {
    let mut output: Option<rodio::OutputStream> = None;
    let mut sink: Option<rodio::Sink> = None;
    let set = |change: &dyn Fn(&mut Status)| {
        if let Ok(mut status) = status.lock() {
            change(&mut status);
        }
    };
    loop {
        let command = match commands.recv_timeout(TICK) {
            Ok(command) => Some(command),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        match command {
            Some(Command::Play(path, track)) => {
                sink = None;
                let started = (|| -> Result<rodio::Sink> {
                    if output.is_none() {
                        let mut stream = rodio::OutputStreamBuilder::open_default_stream()
                            .context("Cannot open the audio output")?;
                        stream.log_on_drop(false);
                        output = Some(stream);
                    }
                    let stream = output.as_ref().context("No audio output")?;
                    let file = fs::File::open(&path)?;
                    let source = rodio::Decoder::builder()
                        .with_data(std::io::BufReader::new(file))
                        .with_byte_len(fs::metadata(&path)?.len())
                        .with_hint("m4a")
                        .with_seekable(true)
                        .build()
                        .context("Cannot read this YouTube Music song")?;
                    let started = rodio::Sink::connect_new(stream.mixer());
                    started.set_volume(gain(status.lock().map_or(100, |status| status.volume)));
                    started.append(source);
                    Ok(started)
                })();
                match started {
                    Ok(started) => {
                        sink = Some(started);
                        set(&|status| {
                            status.track = Some((*track).clone());
                            status.playing = true;
                            status.position_ms = 0;
                        });
                    }
                    Err(error) => {
                        crate::native::diagnostic(&format!(
                            "YouTube Music playback failed: {error:#}"
                        ));
                        set(&|status| status.playing = false);
                    }
                }
            }
            Some(Command::Pause) => {
                if let Some(sink) = &sink {
                    sink.pause();
                }
            }
            Some(Command::Resume) => {
                if let Some(sink) = &sink {
                    sink.play();
                }
            }
            Some(Command::Seek(position_ms)) => {
                if let Some(sink) = &sink {
                    let _ = sink.try_seek(Duration::from_millis(position_ms));
                }
            }
            Some(Command::Volume(percent)) => {
                if let Some(sink) = &sink {
                    sink.set_volume(gain(percent));
                }
                set(&|status| status.volume = percent);
            }
            Some(Command::Stop) => {
                sink = None;
                set(&|status| {
                    status.track = None;
                    status.playing = false;
                    status.position_ms = 0;
                });
            }
            None => {}
        }
        if let Some(current) = &sink {
            let ended = current.empty();
            let playing = !ended && !current.is_paused();
            let position_ms = current.get_pos().as_millis() as u64;
            set(&|status| {
                status.playing = playing;
                if !ended {
                    status.position_ms = position_ms;
                }
            });
        }
    }
}

fn cache_dir() -> Result<PathBuf> {
    Ok(crate::config::data_dir()?.join("ytmusic").join("cache"))
}

fn download(stream: &crate::ytm_tools::Stream, into: &Path) -> Result<()> {
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in &stream.headers {
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(name.as_bytes()),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            headers.insert(name, value);
        }
    }
    let client = reqwest::blocking::Client::builder()
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .build()?;
    let mut file = fs::File::create(into)?;
    let mut offset = 0u64;
    // YouTube slows unranged downloads to playback speed; yt-dlp asks for
    // 10 MB ranges for the same reason.
    loop {
        let end = offset + CHUNK - 1;
        let response = client
            .get(&stream.url)
            .header(reqwest::header::RANGE, format!("bytes={offset}-{end}"))
            .send()
            .context("Cannot reach YouTube Music's audio")?;
        ensure!(
            response.status().is_success(),
            "YouTube Music's audio returned HTTP {}",
            response.status().as_u16()
        );
        let total = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(|range| range.rsplit_once('/'))
            .and_then(|(_, total)| total.parse::<u64>().ok())
            .or(stream.size);
        let mut body = Vec::new();
        response.take(CHUNK + 1).read_to_end(&mut body)?;
        file.write_all(&body)?;
        offset += body.len() as u64;
        ensure!(offset <= MAX_SONG, "This song is too large");
        let done = match total {
            Some(total) => offset >= total,
            None => (body.len() as u64) < CHUNK,
        };
        if done || body.is_empty() {
            break;
        }
    }
    file.sync_all()?;
    ensure!(offset > 0, "YouTube Music sent no audio");
    Ok(())
}

/// The song's audio on disk, fetched first when it is not cached.
pub fn fetch(video_id: &str) -> Result<PathBuf> {
    ensure!(
        crate::ytm_tools::valid_video_id(video_id),
        "Not a YouTube Music song"
    );
    let dir = cache_dir()?;
    let path = dir.join(format!("{video_id}.m4a"));
    if path.is_file() {
        return Ok(path);
    }
    fs::create_dir_all(&dir)?;
    let stream = crate::ytm_tools::resolve(video_id)?;
    let partial = dir.join(format!("{video_id}.download"));
    let result = download(&stream, &partial);
    if let Err(error) = result {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    fs::rename(&partial, &path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_follows_the_slider_squared() {
        assert_eq!(gain(100), 1.0);
        assert_eq!(gain(50), 0.25);
        assert_eq!(gain(0), 0.0);
    }

    #[test]
    fn stopping_hands_playback_back_to_spotify() {
        ACTIVE.store(true, Ordering::Release);
        Player::default().stop();
        assert!(!active());
    }
}

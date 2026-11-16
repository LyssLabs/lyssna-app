//! Plays YouTube Music songs on this computer. Spotify's engine needs a
//! Spotify session, so YouTube Music has its own: the song's AAC stream
//! plays while it downloads into a small cache, decoded by symphonia
//! through rodio.

use anyhow::{Context, Result, ensure};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

use crate::model::Track;

const MAX_SONG: u64 = 200 * 1024 * 1024;
/// Enough audio to start without the decoder catching up with the download.
const START_AFTER: u64 = 256 * 1024;
const CACHE_SONGS: usize = 50;
const CACHE_BYTES: u64 = 500 * 1024 * 1024;
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
    Play(Audio, Box<Track>),
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

    pub fn play(&self, audio: Audio, track: Track) -> Result<()> {
        ACTIVE.store(true, Ordering::Release);
        self.send(Command::Play(audio, Box::new(track)))
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
            Some(Command::Play(audio, track)) => {
                sink = None;
                let started = (|| -> Result<rodio::Sink> {
                    if output.is_none() {
                        let mut stream = rodio::OutputStreamBuilder::open_default_stream()
                            .context("Cannot open the audio output")?;
                        stream.log_on_drop(false);
                        output = Some(stream);
                    }
                    let stream = output.as_ref().context("No audio output")?;
                    let source = rodio::Decoder::builder()
                        .with_byte_len(audio.len)
                        .with_data(std::io::BufReader::new(audio))
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

#[derive(Default)]
struct Progress {
    written: u64,
    total: Option<u64>,
    done: bool,
    failed: bool,
}

type Shared = Arc<(Mutex<Progress>, Condvar)>;

/// A song's audio as it arrives: reads wait for bytes still downloading.
pub struct Audio {
    file: fs::File,
    len: u64,
    progress: Option<Shared>,
    position: u64,
}

impl Read for Audio {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let wanted = match &self.progress {
            None => buffer.len(),
            Some(shared) => {
                let (lock, ready) = &**shared;
                let broken = |_| std::io::Error::other("YouTube Music download unavailable");
                let mut progress = lock.lock().map_err(broken)?;
                while progress.written <= self.position && !progress.done && !progress.failed {
                    progress = ready.wait(progress).map_err(broken)?;
                }
                if progress.failed && progress.written <= self.position {
                    return Err(std::io::Error::other("YouTube Music's audio stopped"));
                }
                (buffer.len() as u64).min(progress.written.saturating_sub(self.position)) as usize
            }
        };
        if wanted == 0 {
            return Ok(0);
        }
        let read = self.file.read(&mut buffer[..wanted])?;
        self.position += read as u64;
        Ok(read)
    }
}

impl Seek for Audio {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let target = match to {
            SeekFrom::Start(offset) => offset,
            SeekFrom::End(offset) => self.len.saturating_add_signed(offset),
            SeekFrom::Current(offset) => self.position.saturating_add_signed(offset),
        };
        self.position = self.file.seek(SeekFrom::Start(target))?;
        Ok(self.position)
    }
}

/// Lets the download rename a file the player still has open.
fn shared_open(options: &mut fs::OpenOptions) -> &mut fs::OpenOptions {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const SHARE_READ_WRITE_DELETE: u32 = 0x1 | 0x2 | 0x4;
        options.share_mode(SHARE_READ_WRITE_DELETE);
    }
    options
}

fn download(
    stream: &crate::ytm_tools::Stream,
    info_file: &Path,
    file: &mut fs::File,
    shared: &Shared,
) -> Result<()> {
    let (lock, ready) = &**shared;
    let update = |change: &dyn Fn(&mut Progress)| {
        if let Ok(mut progress) = lock.lock() {
            change(&mut progress);
        }
        ready.notify_all();
    };
    if let Some(total) = stream.size {
        ensure!(total <= MAX_SONG, "This song is too large");
        update(&|progress| progress.total = Some(total));
    }
    fs::write(info_file, &stream.info)?;
    let mut child = crate::ytm_tools::download_song(info_file)?;
    let mut stdout = child.stdout.take().context("yt-dlp gave no audio")?;
    let mut stderr = child.stderr.take().context("yt-dlp gave no errors")?;
    let errors = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = stderr.read_to_end(&mut text);
        text
    });
    let mut offset = 0u64;
    let mut piece = vec![0; 64 * 1024];
    let copied = (|| -> Result<()> {
        loop {
            let read = stdout.read(&mut piece)?;
            if read == 0 {
                return Ok(());
            }
            file.write_all(&piece[..read])?;
            offset += read as u64;
            ensure!(offset <= MAX_SONG, "This song is too large");
            update(&|progress| progress.written = offset);
        }
    })();
    if copied.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    let errors = errors.join().unwrap_or_default();
    copied?;
    ensure!(
        status.success(),
        "yt-dlp could not download the song: {}",
        crate::ytm_tools::failure(&errors)
    );
    ensure!(offset > 0, "YouTube Music sent no audio");
    file.sync_all()?;
    Ok(())
}

/// Keeps the songs played most recently, within the cache's limits.
fn prune(dir: &Path, keep: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut songs: Vec<(SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "m4a"))
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            Some((meta.modified().ok()?, meta.len(), entry.path()))
        })
        .collect();
    songs.sort_by_key(|song| std::cmp::Reverse(song.0));
    let mut bytes = 0;
    for (index, (_, len, path)) in songs.iter().enumerate() {
        bytes += len;
        if path != keep && (index >= CACHE_SONGS || bytes > CACHE_BYTES) {
            let _ = fs::remove_file(path);
        }
    }
}

fn finish(result: Result<()>, shared: &Shared, partial: &Path, path: &Path) {
    let (lock, ready) = &**shared;
    if let Ok(mut progress) = lock.lock() {
        progress.done = result.is_ok();
        progress.failed = result.is_err();
    }
    ready.notify_all();
    match result {
        Ok(()) => {
            if fs::rename(partial, path).is_ok()
                && let Some(dir) = path.parent()
            {
                prune(dir, path);
            }
        }
        Err(error) => {
            crate::native::diagnostic(&format!("YouTube Music download failed: {error:#}"));
            let _ = fs::remove_file(partial);
        }
    }
}

/// The song's audio, from the cache or streamed while it downloads into it.
pub fn open(video_id: &str) -> Result<Audio> {
    ensure!(
        crate::ytm_tools::valid_video_id(video_id),
        "Not a YouTube Music song"
    );
    let dir = cache_dir()?;
    let path = dir.join(format!("{video_id}.m4a"));
    if let Ok(file) = shared_open(fs::OpenOptions::new().read(true).write(true)).open(&path) {
        let len = file.metadata()?.len();
        let _ = file.set_modified(SystemTime::now());
        return Ok(Audio {
            file,
            len,
            progress: None,
            position: 0,
        });
    }
    fs::create_dir_all(&dir)?;
    let stream = crate::ytm_tools::resolve(video_id)?;
    let partial = dir.join(format!("{video_id}.download"));
    let mut writer = shared_open(
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true),
    )
    .open(&partial)?;
    let reader = shared_open(fs::OpenOptions::new().read(true)).open(&partial)?;
    let shared: Shared = Arc::default();
    let downloading = shared.clone();
    std::thread::Builder::new()
        .name("ytmusic-download".into())
        .spawn(move || {
            let info_file = partial.with_extension("info.json");
            let result = download(&stream, &info_file, &mut writer, &downloading);
            let _ = fs::remove_file(&info_file);
            drop(writer);
            finish(result, &downloading, &partial, &path);
        })?;
    let (lock, ready) = &*shared;
    let broken = |_| anyhow::anyhow!("YouTube Music download unavailable");
    let mut progress = lock.lock().map_err(broken)?;
    // Without the size, seeking from the end needs the whole song first.
    while (progress.written < START_AFTER || progress.total.is_none())
        && !progress.done
        && !progress.failed
    {
        progress = ready.wait(progress).map_err(broken)?;
    }
    ensure!(
        !progress.failed,
        "YouTube Music's audio could not be downloaded"
    );
    let len = progress.total.unwrap_or(progress.written);
    drop(progress);
    Ok(Audio {
        file: reader,
        len,
        progress: Some(shared),
        position: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Temp {
        fn new(name: &str) -> Self {
            Self(std::env::temp_dir().join(format!("lyssna-ytm-{}-{name}", std::process::id())))
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn arriving(name: &str, bytes: &[u8]) -> (Audio, Shared, Temp) {
        let temp = Temp::new(name);
        fs::write(&temp.0, bytes).unwrap();
        let shared: Shared = Arc::default();
        let audio = Audio {
            file: fs::File::open(&temp.0).unwrap(),
            len: bytes.len() as u64,
            progress: Some(shared.clone()),
            position: 0,
        };
        (audio, shared, temp)
    }

    #[test]
    fn volume_follows_the_slider_squared() {
        assert_eq!(gain(100), 1.0);
        assert_eq!(gain(50), 0.25);
        assert_eq!(gain(0), 0.0);
    }

    #[test]
    fn reads_wait_for_audio_still_downloading() {
        let (mut audio, shared, _temp) = arriving("wait", b"abcdef");
        shared.0.lock().unwrap().written = 3;
        let mut buffer = [0; 6];
        assert_eq!(audio.read(&mut buffer).unwrap(), 3);
        assert_eq!(&buffer[..3], b"abc");
        let writer = shared.clone();
        let later = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            writer.0.lock().unwrap().written = 6;
            writer.1.notify_all();
        });
        assert_eq!(audio.read(&mut buffer).unwrap(), 3);
        assert_eq!(&buffer[..3], b"def");
        later.join().unwrap();
        shared.0.lock().unwrap().done = true;
        assert_eq!(audio.read(&mut buffer).unwrap(), 0);
    }

    #[test]
    fn a_failed_download_ends_the_song_with_an_error() {
        let (mut audio, shared, _temp) = arriving("failed", b"abc");
        {
            let mut progress = shared.0.lock().unwrap();
            progress.written = 3;
            progress.failed = true;
        }
        let mut buffer = [0; 8];
        assert_eq!(audio.read(&mut buffer).unwrap(), 3);
        assert!(audio.read(&mut buffer).is_err());
    }

    #[test]
    fn seeking_from_the_end_uses_the_whole_size() {
        let (mut audio, _shared, _temp) = arriving("seek", b"abcdef");
        assert_eq!(audio.seek(SeekFrom::End(-2)).unwrap(), 4);
    }

    #[test]
    fn the_cache_keeps_the_songs_played_last() {
        let dir = Temp::new("cache");
        fs::create_dir_all(&dir.0).unwrap();
        let now = SystemTime::now();
        let song = |index: usize| dir.0.join(format!("{index:011}.m4a"));
        for index in 0..CACHE_SONGS + 2 {
            fs::write(song(index), b"x").unwrap();
            fs::File::options()
                .write(true)
                .open(song(index))
                .unwrap()
                .set_modified(now - Duration::from_secs(index as u64 * 60))
                .unwrap();
        }
        let playing = song(CACHE_SONGS + 1);
        prune(&dir.0, &playing);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), CACHE_SONGS + 1);
        assert!(playing.exists());
        assert!(!song(CACHE_SONGS).exists());
        assert!(song(CACHE_SONGS - 1).exists());
    }

    #[test]
    fn stopping_hands_playback_back_to_spotify() {
        ACTIVE.store(true, Ordering::Release);
        Player::default().stop();
        assert!(!active());
    }
}

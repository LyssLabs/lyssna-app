use std::{collections::BTreeMap, fs, path::{Path, PathBuf}};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{config::{Settings, data_dir}, lyrics::{Lyrics, SavedLyrics}};

const MAX_BYTES: usize = 100 * 1024 * 1024;
pub static TRANSFER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Clone, Serialize, Deserialize)]
pub struct TransferLyrics {
    pub uri: String,
    pub kind: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artists: String,
    #[serde(default)]
    pub image: String,
}

#[derive(Serialize, Deserialize)]
pub struct NativeExport {
    pub settings: Settings,
    pub lyrics: Vec<TransferLyrics>,
    pub preserved: Value,
}

fn valid_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Cannot read {}", path.display())),
    }
}

pub fn export(settings: Settings) -> Result<NativeExport> {
    let directory = data_dir()?;
    let local = directory.join("lyrics").join("local");
    let preserved = read_optional(&directory.join("transfer-preserved.json"))?
        .map(|bytes| serde_json::from_slice(&bytes))
        .transpose().context("Cannot read preserved settings; existing data was kept")?
        .unwrap_or(Value::Null);
    let mut files: BTreeMap<String, String> = BTreeMap::new();
    if local.exists() {
        for entry in fs::read_dir(&local)? {
            let path = entry?.path();
            let Some(kind @ ("ttml" | "lrc" | "json")) = path.extension().and_then(|value| value.to_str()) else { continue };
            let Some(key) = path.file_stem().and_then(|value| value.to_str()).filter(|key| valid_key(key)) else { continue };
            let rank = |kind: &str| match kind { "ttml" => 0, "lrc" => 1, _ => 2 };
            if files.get(key).is_none_or(|previous| rank(kind) < rank(previous)) {
                files.insert(key.into(), kind.into());
            }
        }
    }
    ensure!(files.len() <= 10_000, "Exports support at most 10,000 local songs");
    let mut known = None;
    let mut lyrics = Vec::new();
    let mut total = 0;
    for (key, kind) in files {
        let path = local.join(format!("{key}.{kind}"));
        ensure!(fs::metadata(&path)?.len() <= 2_000_000, "A local lyric file exceeds 2 MB");
        let text = fs::read_to_string(&path).context("Cannot read a saved lyric file as UTF-8")?;
        total += text.len();
        ensure!(total <= MAX_BYTES, "Local lyrics exceed the 100 MB transfer limit");
        let mut metadata: SavedLyrics = read_optional(&local.join(format!("{key}.meta.json")))?
            .map(|bytes| serde_json::from_slice(&bytes)).transpose()
            .context("Cannot read saved lyric metadata")?.unwrap_or_default();
        if metadata.uri.is_empty() && let Some(track) = known.get_or_insert_with(crate::library_cache::known_tracks).get(&key) {
            metadata.uri = track.uri.clone();
            metadata.name = track.name.clone();
            metadata.artists = track.artist();
        }
        lyrics.push(TransferLyrics { uri: metadata.uri, kind, text, key: Some(key), name: metadata.name, artists: metadata.artists, image: metadata.image });
    }
    Ok(NativeExport { settings, lyrics, preserved })
}

pub fn backup() -> Result<Option<NativeExport>> {
    read_optional(&data_dir()?.join("transfer-backup.json"))?
        .map(|bytes| serde_json::from_slice(&bytes).context("Cannot read the previous import backup"))
        .transpose()
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    let temporary = path.with_extension("transfer-tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path).with_context(|| format!("Cannot save {}", path.display()))
}

struct Change { path: PathBuf, previous: Option<Vec<u8>>, next: Option<Vec<u8>> }

pub struct ImportTransaction { changes: Vec<Change>, committed: bool }

impl ImportTransaction {
    pub fn begin(before: Settings, lyrics: Vec<TransferLyrics>, preserved: Value, window_preferences: BTreeMap<String, String>) -> Result<Self> {
        ensure!(lyrics.len() <= 10_000, "Import supports at most 10,000 local songs");
        let directory = data_dir()?;
        let local = directory.join("lyrics").join("local");
        let preserved = serde_json::to_vec(&preserved)?;
        ensure!(preserved.len() <= 16 * 1024 * 1024, "Preserved preferences exceed 16 MB");
        let mut pending: BTreeMap<PathBuf, Option<Vec<u8>>> = BTreeMap::new();
        pending.insert(directory.join("transfer-preserved.json"), Some(preserved));
        let mut seen = std::collections::HashSet::new();
        let mut total = 0;
        for entry in lyrics {
            total += entry.text.len();
            ensure!(total <= MAX_BYTES && entry.text.len() <= 2_000_000, "Lyric files exceed transfer limits");
            ensure!(entry.uri.len() <= 512 && !entry.uri.chars().any(char::is_control), "Invalid track URI");
            ensure!(entry.name.len() <= 16_384 && entry.artists.len() <= 16_384 && entry.image.len() <= 16_384, "Invalid lyric metadata");
            let key = if entry.uri.is_empty() {
                entry.key.filter(|key| valid_key(key)).context("Lyric file is missing its track association")?
            } else { format!("{:x}", Sha256::digest(entry.uri.as_bytes())) };
            ensure!(seen.insert(key.clone()), "Duplicate lyric track in the import");
            match entry.kind.as_str() {
                "ttml" => { crate::lyrics::parse_ttml(&entry.text, 0).context("Invalid TTML in the import")?; }
                "lrc" => { crate::lyrics::parse_lrc(&entry.text, 0).context("Invalid LRC in the import")?; }
                "json" => { serde_json::from_str::<Lyrics>(&entry.text).context("Invalid legacy lyrics in the import")?; }
                _ => anyhow::bail!("Unsupported lyric format"),
            }
            for kind in ["ttml", "lrc", "json"] {
                pending.insert(local.join(format!("{key}.{kind}")), if kind == entry.kind { Some(entry.text.as_bytes().to_vec()) } else { None });
            }
            let meta_path = local.join(format!("{key}.meta.json"));
            let mut metadata: SavedLyrics = read_optional(&meta_path)?
                .map(|bytes| serde_json::from_slice(&bytes)).transpose().context("Cannot preserve existing lyric metadata")?.unwrap_or_default();
            metadata.key = key;
            metadata.kind = entry.kind;
            if !entry.uri.is_empty() { metadata.uri = entry.uri; }
            if !entry.name.is_empty() { metadata.name = entry.name; }
            if !entry.artists.is_empty() { metadata.artists = entry.artists; }
            if !entry.image.is_empty() { metadata.image = entry.image; }
            metadata.saved_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
            pending.insert(meta_path, Some(serde_json::to_vec(&metadata)?));
        }
        let mut changes = Vec::new();
        for (path, next) in pending {
            changes.push(Change { previous: read_optional(&path)?, path, next });
        }
        // The settings writer runs after the lyric files, inside this transaction.
        let settings_path = directory.join("settings.json");
        changes.push(Change { previous: read_optional(&settings_path)?, path: settings_path, next: None });
        ensure!(window_preferences.keys().all(|key| key.starts_with("lyssna.")), "Invalid window preferences");
        ensure!(serde_json::to_vec(&window_preferences)?.len() <= 16 * 1024 * 1024, "Window preferences exceed 16 MB");
        let mut backup = export(before)?;
        if !backup.preserved.is_object() { backup.preserved = serde_json::json!({}); }
        if !backup.preserved["standalone"].is_object() { backup.preserved["standalone"] = serde_json::json!({}); }
        backup.preserved["standalone"]["storage"] = serde_json::to_value(window_preferences)?;
        write_atomic(&directory.join("transfer-backup.json"), &serde_json::to_vec(&backup)?)?;
        let mut transaction = Self { changes, committed: false };
        let result = (|| -> Result<()> {
            for change in transaction.changes.iter().filter(|change| change.path.file_name().is_none_or(|name| name != "settings.json")) {
                if let Some(bytes) = &change.next { write_atomic(&change.path, bytes)?; }
                else if change.previous.is_some() { fs::remove_file(&change.path)?; }
            }
            Ok(())
        })();
        if let Err(error) = result {
            transaction.rollback().context("Import failed and automatic recovery was incomplete; download the previous backup")?;
            return Err(error);
        }
        Ok(transaction)
    }

    pub fn rollback(&mut self) -> Result<()> {
        let mut failure = None;
        for change in self.changes.iter().rev() {
            let result = match &change.previous {
                Some(bytes) => write_atomic(&change.path, bytes),
                None => match fs::remove_file(&change.path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error.into()),
                },
            };
            if let Err(error) = result { failure = Some(error); }
        }
        if let Some(error) = failure { return Err(error); }
        self.committed = true;
        Ok(())
    }

    pub fn commit(mut self) { self.committed = true; }
}

impl Drop for ImportTransaction {
    fn drop(&mut self) {
        if !self.committed && let Err(error) = self.rollback() {
            log::error!("Settings import recovery failed: {error:#}");
        }
    }
}

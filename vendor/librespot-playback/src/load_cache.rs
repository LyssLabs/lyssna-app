//! Bounded, session-owned startup data. Audio keys never reach disk.
use crate::audio::AudioFile;
use crate::core::{FileId, SpotifyId, SpotifyUri, audio_key::AudioKey};
use crate::metadata::audio::AudioItem;
use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};

const CAPACITY: usize = 64;
const LIFETIME: Duration = Duration::from_secs(300);

pub(crate) struct TimedCache<K, V> {
    entries: Mutex<VecDeque<(K, Instant, V)>>,
}
impl<K, V> Default for TimedCache<K, V> {
    fn default() -> Self {
        Self {
            entries: Mutex::default(),
        }
    }
}
impl<K: PartialEq, V: Clone> TimedCache<K, V> {
    pub fn get(&self, key: &K, now: Instant) -> Option<V> {
        let mut entries = self.entries.lock().ok()?;
        let index = entries
            .iter()
            .position(|(candidate, _, _)| candidate == key)?;
        let entry = entries.remove(index)?;
        if now.saturating_duration_since(entry.1) >= LIFETIME {
            return None;
        }
        let value = entry.2.clone();
        entries.push_back(entry);
        Some(value)
    }
    pub fn insert(&self, key: K, value: V, now: Instant) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|(old, inserted, _)| {
                old != &key && now.saturating_duration_since(*inserted) < LIFETIME
            });
            while entries.len() >= CAPACITY {
                entries.pop_front();
            }
            entries.push_back((key, now, value));
        }
    }
}

#[derive(Default)]
pub(crate) struct LoadCache {
    pub metadata: TimedCache<SpotifyUri, AudioItem>,
    pub keys: TimedCache<(SpotifyId, FileId), AudioKey>,
    pub warm_files: WarmFiles,
}

const WARM_CAPACITY: usize = 3;
const WARM_LIFETIME: Duration = Duration::from_secs(120);

/// Audio files opened ahead of a skip. Each holds a live download, so only a
/// few are kept and each is handed out once.
#[derive(Default)]
pub(crate) struct WarmFiles {
    entries: Mutex<VecDeque<(FileId, Instant, AudioFile)>>,
}
impl WarmFiles {
    pub fn contains(&self, file: FileId) -> bool {
        self.entries
            .lock()
            .is_ok_and(|entries| entries.iter().any(|(id, _, _)| *id == file))
    }
    pub fn take(&self, file: FileId, now: Instant) -> Option<AudioFile> {
        let mut entries = self.entries.lock().ok()?;
        let index = entries.iter().position(|(id, _, _)| *id == file)?;
        let (_, opened, audio) = entries.remove(index)?;
        (now.saturating_duration_since(opened) < WARM_LIFETIME).then_some(audio)
    }
    pub fn insert(&self, file: FileId, audio: AudioFile, now: Instant) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|(id, opened, _)| {
                *id != file && now.saturating_duration_since(*opened) < WARM_LIFETIME
            });
            while entries.len() >= WARM_CAPACITY {
                entries.pop_front();
            }
            entries.push_back((file, now, audio));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_reads_do_not_extend_authorization_data_lifetime() {
        let now = Instant::now();
        let cache = TimedCache::default();
        cache.insert(1, "value", now);
        assert_eq!(cache.get(&1, now + LIFETIME / 2), Some("value"));
        assert_eq!(cache.get(&1, now + LIFETIME), None);
    }
    #[test]
    fn bounded_cache_preserves_recently_used_entries() {
        let now = Instant::now();
        let cache = TimedCache::default();
        for value in 0..CAPACITY {
            cache.insert(value, value, now);
        }
        assert_eq!(cache.get(&0, now), Some(0));
        cache.insert(CAPACITY, CAPACITY, now);
        assert_eq!(cache.get(&1, now), None);
        assert_eq!(cache.get(&0, now), Some(0));
        assert_eq!(cache.entries.lock().unwrap().len(), CAPACITY);
    }
    #[test]
    fn new_sessions_do_not_share_startup_data() {
        let now = Instant::now();
        let first = TimedCache::default();
        first.insert(1, "private value", now);
        let second = TimedCache::<i32, &str>::default();
        assert_eq!(second.get(&1, now), None);
    }
}

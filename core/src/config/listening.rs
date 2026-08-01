//! Local, on-disk listening-progress store (audiobook playback positions).
//!
//! This is the audio-playback sibling of [`super::reading::ReadingStore`]. The
//! two are kept as **parallel stores** rather than one shared file on purpose:
//!   * a reading position is an opaque *text* locator (EPUB CFI) plus a percent,
//!   * a listening position is a wall-clock *offset in seconds* plus a percent,
//!
//! and a single catalog item can be *both* an ebook and an audiobook (an ABS
//! item often is). Separate files let a book hold an independent reading and
//! listening position without one clobbering the other, while reusing the exact
//! same atomic-write (temp file + rename) safety as the reading store.
//!
//! Positions are stored as [`Progress`] keyed by `book_id`, in
//! `listening.json` at the platform data location. Pure-client and on-device:
//! there is no progress server. A later phase will sync these positions to
//! Audiobookshelf's progress API (analogous to the Hardcover reading sync) and
//! encrypt this file at rest — see the TODOs.

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use crate::models::Progress;

use super::ConfigError;

/// A file-backed store of per-book **listening** (audiobook) progress.
pub struct ListeningStore {
    path: PathBuf,
}

impl ListeningStore {
    /// Create a store backed by an explicit file path (used by tests and by the
    /// Tauri layer, which passes a platform data path).
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The default store at the platform data location
    /// (`%APPDATA%/Libro/listening.json`, `$XDG_CONFIG_HOME/Libro/listening.json`, …).
    pub fn default_store() -> Self {
        Self::new(super::data_dir().join("listening.json"))
    }

    /// Read the whole map, treating a missing/corrupt file as an empty store.
    fn load_map(&self) -> Result<BTreeMap<String, Progress>, ConfigError> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(ConfigError::Io(e.to_string())),
        }
    }

    /// Write the whole map atomically (temp file + rename).
    fn save_map(&self, map: &BTreeMap<String, Progress>) -> Result<(), ConfigError> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| ConfigError::Io(e.to_string()))?;
            }
        }
        let bytes = serde_json::to_vec_pretty(map)?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, &bytes).map_err(|e| ConfigError::Io(e.to_string()))?;
        fs::rename(&tmp, &self.path).map_err(|e| ConfigError::Io(e.to_string()))?;
        // TODO(security): encrypt `bytes` at rest with the keychain-held key,
        // mirroring crate::config, and reconcile with the device-to-device backup.
        Ok(())
    }

    /// Fetch the stored listening progress for `book_id`, if any.
    pub fn get(&self, book_id: &str) -> Result<Option<Progress>, ConfigError> {
        Ok(self.load_map()?.remove(book_id))
    }

    /// Persist (insert or replace) the listening progress for `book_id`.
    pub fn save(&self, book_id: &str, progress: Progress) -> Result<(), ConfigError> {
        let mut map = self.load_map()?;
        map.insert(book_id.to_string(), progress);
        self.save_map(&map)
    }
}

/// Inbound reconciliation write surface (see [`crate::progress_sync`]). Read
/// errors are treated as "no local value"; write errors surface as a `String`.
impl crate::progress_sync::ProgressStoreLike for ListeningStore {
    fn get_progress(&self, key: &str) -> Option<Progress> {
        self.get(key).ok().flatten()
    }
    fn put_progress(&self, key: &str, value: Progress) -> Result<(), String> {
        self.save(key, value).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio_progress(fraction: f32, seconds: f64, finished: bool) -> Progress {
        Progress {
            fraction,
            position_seconds: Some(seconds),
            locator: None,
            finished,
        }
    }

    #[test]
    fn save_then_get_round_trips_seconds_and_fraction() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ListeningStore::new(tmp.path().join("listening.json"));

        assert!(store.get("audio-1").unwrap().is_none());

        let p = audio_progress(0.25, 632.56, false);
        store.save("audio-1", p.clone()).unwrap();

        let got = store.get("audio-1").unwrap().expect("saved progress");
        assert_eq!(got, p);
        assert_eq!(got.position_seconds, Some(632.56));
        assert!((got.fraction - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn save_overwrites_previous_and_keeps_other_books() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ListeningStore::new(tmp.path().join("listening.json"));

        store.save("a", audio_progress(0.1, 60.0, false)).unwrap();
        store.save("b", audio_progress(0.2, 120.0, false)).unwrap();
        // Overwrite a with a later position.
        store.save("a", audio_progress(1.0, 3600.0, true)).unwrap();

        let a = store.get("a").unwrap().unwrap();
        assert!(a.finished);
        assert_eq!(a.position_seconds, Some(3600.0));
        // b is untouched.
        let b = store.get("b").unwrap().unwrap();
        assert_eq!(b.position_seconds, Some(120.0));
    }

    #[test]
    fn missing_file_is_an_empty_store_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ListeningStore::new(tmp.path().join("does-not-exist.json"));
        assert!(store.get("anything").unwrap().is_none());
    }

    #[test]
    fn reading_and_listening_positions_do_not_collide() {
        // The two stores are independent files, so the same book_id can hold a
        // distinct reading (locator) and listening (seconds) position.
        let tmp = tempfile::tempdir().unwrap();
        let listening = ListeningStore::new(tmp.path().join("listening.json"));
        let reading = super::super::reading::ReadingStore::new(tmp.path().join("reading.json"));

        listening.save("same-id", audio_progress(0.5, 900.0, false)).unwrap();
        reading
            .save(
                "same-id",
                Progress {
                    fraction: 0.1,
                    position_seconds: None,
                    locator: Some("epubcfi(/6/4)".into()),
                    finished: false,
                },
            )
            .unwrap();

        let l = listening.get("same-id").unwrap().unwrap();
        let r = reading.get("same-id").unwrap().unwrap();
        assert_eq!(l.position_seconds, Some(900.0));
        assert_eq!(r.locator.as_deref(), Some("epubcfi(/6/4)"));
    }
}

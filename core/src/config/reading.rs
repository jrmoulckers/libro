//! Local, on-disk reading-progress store.
//!
//! Reading positions are small, frequently-updated, per-book records, so they
//! live in their own JSON file rather than the (stubbed, encrypted) [`AppConfig`]
//! blob. The store maps `book_id -> `[`Progress`] and is written atomically
//! (temp file + rename) so a crash mid-write can't corrupt it.
//!
//! Pure-client, on-device: there is no progress server. A later phase will sync
//! these positions to reading-tracker connectors (Hardcover / Audiobookshelf) and
//! encrypt this file the same way [`crate::config`] describes — see the TODOs.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use crate::models::Progress;

use super::ConfigError;

/// A file-backed store of per-book reading progress.
pub struct ReadingStore {
    path: PathBuf,
}

impl ReadingStore {
    /// Create a store backed by an explicit file path (used by tests and by the
    /// Tauri layer, which passes a platform data path).
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The default store at the platform data location
    /// (`%APPDATA%/Libro/reading.json`, `$XDG_CONFIG_HOME/Libro/reading.json`, …).
    pub fn default_store() -> Self {
        Self::new(default_path())
    }

    /// Read the whole map, treating a missing file as an empty store.
    fn load_map(&self) -> Result<BTreeMap<String, Progress>, ConfigError> {
        match fs::read(&self.path) {
            Ok(bytes) => {
                // A corrupt/empty file degrades to an empty store rather than a
                // hard error, so a bad write never bricks resume.
                Ok(serde_json::from_slice(&bytes).unwrap_or_default())
            }
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

    /// Fetch the stored progress for `book_id`, if any.
    pub fn get(&self, book_id: &str) -> Result<Option<Progress>, ConfigError> {
        Ok(self.load_map()?.remove(book_id))
    }

    /// Persist (insert or replace) the progress for `book_id`.
    pub fn save(&self, book_id: &str, progress: Progress) -> Result<(), ConfigError> {
        let mut map = self.load_map()?;
        map.insert(book_id.to_string(), progress);
        self.save_map(&map)
    }
}

/// Platform data path for the reading-progress file, using only std env vars so
/// no extra dependency is pulled in.
fn default_path() -> PathBuf {
    base_dir().join("Libro").join("reading.json")
}

/// Platform data base directory, shared with the listening store. Uses only std
/// env vars so no extra dependency is pulled in.
pub(super) fn base_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(v) = env::var("APPDATA") {
            if !v.is_empty() {
                return PathBuf::from(v);
            }
        }
    }
    if let Ok(v) = env::var("XDG_CONFIG_HOME") {
        if !v.is_empty() {
            return PathBuf::from(v);
        }
    }
    if let Ok(v) = env::var("HOME") {
        if !v.is_empty() {
            return PathBuf::from(v).join(".config");
        }
    }
    if let Ok(v) = env::var("USERPROFILE") {
        if !v.is_empty() {
            return PathBuf::from(v);
        }
    }
    env::temp_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(fraction: f32, locator: &str, finished: bool) -> Progress {
        Progress {
            fraction,
            position_seconds: None,
            locator: Some(locator.to_string()),
            finished,
        }
    }

    #[test]
    fn save_then_get_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ReadingStore::new(tmp.path().join("reading.json"));

        assert!(store.get("book-1").unwrap().is_none());

        let p = progress(0.42, "epubcfi(/6/14!/4/2/2)", false);
        store.save("book-1", p.clone()).unwrap();

        let got = store.get("book-1").unwrap().expect("saved progress");
        assert_eq!(got, p);
        assert_eq!(got.locator.as_deref(), Some("epubcfi(/6/14!/4/2/2)"));
        assert!((got.fraction - 0.42).abs() < f32::EPSILON);
    }

    #[test]
    fn save_overwrites_previous_and_keeps_other_books() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ReadingStore::new(tmp.path().join("reading.json"));

        store.save("a", progress(0.1, "loc-a", false)).unwrap();
        store.save("b", progress(0.2, "loc-b", false)).unwrap();
        // Overwrite a.
        store.save("a", progress(1.0, "loc-a2", true)).unwrap();

        let a = store.get("a").unwrap().unwrap();
        assert!(a.finished);
        assert_eq!(a.locator.as_deref(), Some("loc-a2"));
        // b is untouched.
        let b = store.get("b").unwrap().unwrap();
        assert_eq!(b.locator.as_deref(), Some("loc-b"));
    }

    #[test]
    fn missing_file_is_an_empty_store_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ReadingStore::new(tmp.path().join("does-not-exist.json"));
        assert!(store.get("anything").unwrap().is_none());
    }
}

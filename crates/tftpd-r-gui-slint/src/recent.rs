//! Recently used root directories for the status-bar drop-down.
//!
//! The list is small (at most [`MAX_RECENT_DIRS`]) and stored as one path per
//! line in a per-user state file. Persistence is best effort: the panel must
//! still work in read-only or locked-down environments, so write failures are
//! intentionally ignored instead of blocking the UI.

use std::path::{Path, PathBuf};

use crate::user_state::{state_file, RECENT_DIRS_FILE};

/// Maximum number of directories kept in the drop-down.
pub const MAX_RECENT_DIRS: usize = 8;

/// Recently used TFTP root directories, most recently used first.
pub struct RecentDirs {
    paths: Vec<PathBuf>,
    storage: Option<PathBuf>,
}

impl RecentDirs {
    /// Load the persisted list. A missing or unreadable state file yields an
    /// empty list; directories that no longer exist are dropped.
    #[must_use]
    pub fn load() -> Self {
        Self::load_from(state_file(RECENT_DIRS_FILE))
    }

    /// Directories in most-recently-used order.
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Empty history without a storage file, for tests in other modules.
    #[cfg(test)]
    #[must_use]
    pub fn empty_for_tests() -> Self {
        Self {
            paths: Vec::new(),
            storage: None,
        }
    }

    /// Record `path` as the most recently used directory.
    ///
    /// Returns `false` when `path` does not exist or is not a directory. The
    /// path is stored canonicalized so duplicate spellings collapse and the
    /// drop-down always shows an absolute path.
    pub fn record(&mut self, path: &Path) -> bool {
        let Ok(path) = std::fs::canonicalize(path) else {
            return false;
        };
        if !path.is_dir() {
            return false;
        }
        self.insert(path);
        self.save();
        true
    }

    fn load_from(storage: Option<PathBuf>) -> Self {
        let mut recent = Self {
            paths: Vec::new(),
            storage,
        };
        if let Some(storage) = &recent.storage {
            if let Ok(text) = std::fs::read_to_string(storage) {
                // The file is most-recent-first; insert in reverse so the
                // order survives the front insertion. Canonicalise on load so
                // the same directory written with a different spelling (for
                // example with a trailing separator or without the Windows
                // verbatim prefix) does not appear twice.
                for line in text.lines().rev() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let Ok(path) = std::fs::canonicalize(line) else {
                        continue;
                    };
                    if path.is_dir() {
                        recent.insert(path);
                    }
                }
            }
        }
        recent
    }

    /// Forget every recorded directory and remove the persisted list. The
    /// configured/running root is not affected: it is server configuration, not
    /// history.
    pub fn clear(&mut self) {
        self.paths.clear();
        if let Some(storage) = &self.storage {
            // Best effort like `save`: a read-only profile must not break the
            // panel. A missing file is already the desired state.
            let _ = std::fs::remove_file(storage);
        }
    }

    fn insert(&mut self, path: PathBuf) {
        self.paths.retain(|existing| !same_dir(existing, &path));
        self.paths.insert(0, path);
        self.paths.truncate(MAX_RECENT_DIRS);
    }

    fn save(&self) {
        let Some(storage) = &self.storage else {
            return;
        };
        if let Some(parent) = storage.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut text = String::new();
        for path in &self.paths {
            text.push_str(&path.to_string_lossy());
            text.push('\n');
        }
        // Best effort: a read-only profile directory must not break the panel.
        let _ = std::fs::write(storage, text);
    }
}

/// Windows paths are case-insensitive; other platforms compare exactly.
fn same_dir(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).expect("canonical path")
    }

    fn load_from(storage: &Path) -> RecentDirs {
        RecentDirs::load_from(Some(storage.to_path_buf()))
    }

    #[test]
    fn record_keeps_most_recent_first_and_deduplicates() {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage = temp.path().join("recent.txt");
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::create_dir(&first).expect("first dir");
        std::fs::create_dir(&second).expect("second dir");

        let mut recent = load_from(&storage);
        assert!(recent.record(&first));
        assert!(recent.record(&second));
        // Recording the first directory again moves it back to the front.
        assert!(recent.record(&first));

        assert_eq!(recent.paths(), &[canonical(&first), canonical(&second)][..]);
    }

    #[test]
    fn record_caps_the_list_at_the_configured_maximum() {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage = temp.path().join("recent.txt");
        let mut recent = load_from(&storage);
        for index in 0..(MAX_RECENT_DIRS + 3) {
            let dir = temp.path().join(format!("dir-{index}"));
            std::fs::create_dir(&dir).expect("dir");
            assert!(recent.record(&dir));
        }
        assert_eq!(recent.paths().len(), MAX_RECENT_DIRS);
        // The last recorded directory is at the front, the oldest ones fell off.
        assert_eq!(
            recent.paths()[0],
            canonical(&temp.path().join(format!("dir-{}", MAX_RECENT_DIRS + 2)))
        );
    }

    #[test]
    fn persisted_list_reloads_in_the_same_order() {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage = temp.path().join("recent.txt");
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::create_dir(&first).expect("first dir");
        std::fs::create_dir(&second).expect("second dir");

        {
            let mut recent = load_from(&storage);
            assert!(recent.record(&first));
            assert!(recent.record(&second));
        }

        let reloaded = load_from(&storage);
        assert_eq!(
            reloaded.paths(),
            &[canonical(&second), canonical(&first)][..]
        );
    }

    #[test]
    fn missing_or_non_directory_paths_are_not_recorded() {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage = temp.path().join("recent.txt");
        let file = temp.path().join("file.txt");
        std::fs::write(&file, b"not a directory").expect("file");

        let mut recent = load_from(&storage);
        assert!(!recent.record(&temp.path().join("does-not-exist")));
        assert!(!recent.record(&file));
        assert!(recent.paths().is_empty());
    }

    #[test]
    fn stale_directories_are_dropped_when_loading() {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage = temp.path().join("recent.txt");
        let gone = temp.path().join("gone");
        std::fs::create_dir(&gone).expect("gone dir");
        let mut recent = load_from(&storage);
        assert!(recent.record(&gone));
        std::fs::remove_dir(&gone).expect("remove dir");

        let reloaded = load_from(&storage);
        assert!(reloaded.paths().is_empty());
    }

    #[test]
    fn clear_forgets_the_history_and_removes_the_state_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage = temp.path().join("recent.txt");
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::create_dir(&first).expect("first dir");
        std::fs::create_dir(&second).expect("second dir");

        let mut recent = load_from(&storage);
        assert!(recent.record(&first));
        assert!(recent.record(&second));
        assert!(storage.exists());

        recent.clear();
        assert!(recent.paths().is_empty());
        assert!(
            !storage.exists(),
            "clearing the history must remove the state file"
        );
        assert!(load_from(&storage).paths().is_empty());
    }

    #[test]
    fn loading_normalises_duplicate_spellings_of_the_same_directory() {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage = temp.path().join("recent.txt");
        let dir = temp.path().join("root");
        std::fs::create_dir(&dir).expect("dir");

        let canonical = canonical(&dir);
        // A trailing separator is the same directory but a different string;
        // the Windows canonical form also differs from `dir.display()`.
        let decorated = format!("{}{}", dir.display(), std::path::MAIN_SEPARATOR);
        std::fs::write(&storage, format!("{decorated}\n{}\n", canonical.display()))
            .expect("write state");

        let reloaded = load_from(&storage);
        assert_eq!(reloaded.paths(), &[canonical][..]);
    }
}

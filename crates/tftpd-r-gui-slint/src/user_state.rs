//! Per-user state files for GUI-only preferences.
//!
//! The files live next to each other in the current user's profile. They hold
//! advisory UI state (recent root history and one-time setup flags), never
//! server configuration, and are never read by the TFTP server. Unreadable or
//! unwritable state must not prevent the panel from starting, so every write is
//! best effort.

use std::path::PathBuf;

/// File with the canonicalised, most-recently-used root directories.
pub const RECENT_DIRS_FILE: &str = "recent-dirs.txt";
/// File with one-time setup flags.
pub const SETUP_STATE_FILE: &str = "ui-state.txt";

/// Path of a GUI state file in the current user's profile, or `None` when no
/// profile directory can be determined.
#[must_use]
pub fn state_file(name: &str) -> Option<PathBuf> {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return Some(PathBuf::from(appdata).join("TftpdR").join(name));
    }
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(config_home).join("tftpd-r").join(name));
    }
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".config")
            .join("tftpd-r")
            .join(name)
    })
}

/// One-time setup state persisted for the current user.
///
/// The first launch of the panel opens the settings dialog so the operator can
/// choose the root directory and bind addresses; this flag makes that happen
/// once instead of on every unconfigured launch.
pub struct SetupState {
    storage: Option<PathBuf>,
    shown: bool,
}

impl SetupState {
    /// Load the flags for the current user.
    #[must_use]
    pub fn load() -> Self {
        Self::load_from(state_file(SETUP_STATE_FILE))
    }

    /// Whether the setup dialog still has to be shown.
    #[must_use]
    pub fn should_show_setup(&self) -> bool {
        !self.shown
    }

    /// Remember that the setup dialog was shown. Best effort: the in-memory
    /// flag is set even when the profile directory is read-only.
    pub fn mark_setup_shown(&mut self) {
        if self.shown {
            return;
        }
        self.shown = true;
        self.save();
    }

    fn load_from(storage: Option<PathBuf>) -> Self {
        let shown = storage
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .is_some_and(|text| text.lines().any(|line| line.trim() == "setup-shown=true"));
        Self { storage, shown }
    }

    fn save(&self) {
        let Some(storage) = &self.storage else {
            return;
        };
        if let Some(parent) = storage.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Best effort: a read-only profile must not break the panel.
        let _ = std::fs::write(storage, "setup-shown=true\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_path(dir: &std::path::Path) -> PathBuf {
        dir.join(SETUP_STATE_FILE)
    }

    #[test]
    fn missing_state_file_means_the_setup_dialog_is_due() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = SetupState::load_from(Some(state_path(temp.path())));
        assert!(state.should_show_setup());
    }

    #[test]
    fn marking_the_setup_shown_survives_a_reload() {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage = state_path(temp.path());
        let mut state = SetupState::load_from(Some(storage.clone()));
        state.mark_setup_shown();
        assert!(!state.should_show_setup());

        let reloaded = SetupState::load_from(Some(storage));
        assert!(!reloaded.should_show_setup());
    }

    #[test]
    fn unrelated_or_unreadable_state_still_prompts() {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage = state_path(temp.path());
        std::fs::write(&storage, "some-other-flag=1\n").expect("write state");
        assert!(SetupState::load_from(Some(storage)).should_show_setup());

        // A directory where a file is expected cannot be read as a state file;
        // it must be treated as "first launch", not as an error.
        let directory = temp.path().join("state-dir");
        std::fs::create_dir(&directory).expect("state dir");
        assert!(SetupState::load_from(Some(directory)).should_show_setup());
    }

    #[test]
    fn a_read_only_profile_does_not_panic_when_marking() {
        let temp = tempfile::tempdir().expect("temp dir");
        let parent_file = temp.path().join("not-a-directory");
        std::fs::write(&parent_file, b"x").expect("write parent file");
        let mut state = SetupState::load_from(Some(parent_file.join(SETUP_STATE_FILE)));
        state.mark_setup_shown();
        // The in-memory flag still suppresses a second dialog in this run.
        assert!(!state.should_show_setup());
    }
}

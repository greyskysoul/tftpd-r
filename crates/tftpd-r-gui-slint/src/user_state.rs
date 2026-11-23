//! State files for GUI-only preferences.
//!
//! The files live together in the per-user configuration directory when it is
//! writable. A restricted profile or a low-integrity/sandboxed launch can make
//! that directory unusable; in that case a portable `.tftpd-r` directory beside
//! the executable (or in the working directory) is used instead. The files hold
//! advisory UI state (recent root history, the last used root and one-time setup
//! flags), never server configuration, and are never read by the TFTP server.
//! Unreadable or unwritable state must not prevent the panel from starting, so
//! writes remain best effort; the remembered-root path reports failures so the
//! operator is not left guessing.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::i18n::LanguagePreference;
use crate::theme::ThemeMode;

/// File with the canonicalised, most-recently-used root directories.
pub const RECENT_DIRS_FILE: &str = "recent-dirs.txt";
/// File with the root directory of the last successful start.
pub const LAST_ROOT_FILE: &str = "last-root.txt";
/// File with one-time setup flags.
pub const SETUP_STATE_FILE: &str = "ui-state.txt";
/// File with the panel's display preferences (the theme choice and the
/// language preference).
pub const UI_PREFS_FILE: &str = "ui-prefs.txt";

/// Portable state directory used when the per-user configuration directory
/// cannot be created (for example in a low-integrity or otherwise restricted
/// launch). It lives next to the executable first, then in the working
/// directory.
const PORTABLE_STATE_DIR: &str = ".tftpd-r";

/// Selected state directory and whether it is the portable fallback.
struct StateLocation {
    dir: PathBuf,
    portable: bool,
}

/// Per-user configuration directory for GUI state, or `None` when no profile
/// directory can be determined.
fn primary_state_dir() -> Option<PathBuf> {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return Some(PathBuf::from(appdata).join("TftpdR"));
    }
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(config_home).join("tftpd-r"));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config").join("tftpd-r"))
}

/// Candidate state directories in preference order: per-user profile, then a
/// portable directory beside the executable, then one in the working directory.
fn candidate_state_dirs() -> Vec<(PathBuf, bool)> {
    let mut candidates = Vec::new();
    if let Some(primary) = primary_state_dir() {
        candidates.push((primary, false));
    }
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        candidates.push((exe_dir.join(PORTABLE_STATE_DIR), true));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push((cwd.join(PORTABLE_STATE_DIR), true));
    }
    candidates
}

/// Whether the directory can be created and an actual file can be written in
/// it. Checking only `create_dir_all` is not enough: an existing directory may
/// still deny writes (for example when a restricted process reuses a
/// higher-integrity profile), which would otherwise select it and fail later.
fn directory_is_writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(".write-test-{}", std::process::id()));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Pick the first candidate that can actually be created and written. When none
/// can, the first candidate is kept so later writes report a real error instead
/// of silently pretending the state was saved.
fn select_state_dir(candidates: Vec<(PathBuf, bool)>) -> Option<StateLocation> {
    for (dir, portable) in &candidates {
        if directory_is_writable(dir) {
            return Some(StateLocation {
                dir: dir.clone(),
                portable: *portable,
            });
        }
    }
    candidates
        .into_iter()
        .next()
        .map(|(dir, portable)| StateLocation { dir, portable })
}

fn state_location() -> Option<&'static StateLocation> {
    static LOCATION: OnceLock<Option<StateLocation>> = OnceLock::new();
    LOCATION
        .get_or_init(|| select_state_dir(candidate_state_dirs()))
        .as_ref()
}

/// Path of a GUI state file, creating the chosen state directory on first use.
///
/// The per-user configuration directory is preferred. When it cannot be
/// created (restricted profile, sandboxed/low-integrity launch), the panel uses
/// a portable `.tftpd-r` directory beside the executable (or in the working
/// directory) instead, so remembering the root and the recent list still work.
#[must_use]
pub fn state_file(name: &str) -> Option<PathBuf> {
    state_location().map(|location| location.dir.join(name))
}

/// Read one state file as text, removing a UTF-8 byte-order mark.
///
/// The panel writes these files without a BOM, but an operator editing them in
/// a Windows editor can add one; without stripping it the first line would
/// never match its key and the stored value would silently be ignored. Shared
/// with `recent.rs`, which stores one path per line.
pub(crate) fn read_state_text(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.trim_start_matches('\u{feff}').to_string())
}

/// The portable fallback directory when the per-user directory was not usable.
#[must_use]
pub fn fallback_state_dir() -> Option<&'static Path> {
    state_location()
        .filter(|location| location.portable)
        .map(|location| location.dir.as_path())
}

/// Root directory remembered from the previous panel session.
///
/// The list of recent roots is history and can be cleared; this value is the
/// directory the server was actually started with (or the operator explicitly
/// saved), so the next launch can preselect it without resurrecting every
/// browsed directory. It is still advisory: an entry that no longer exists is
/// dropped on load and the panel opens with no root selected.
pub struct LastRoot {
    storage: Option<PathBuf>,
    path: Option<PathBuf>,
    /// Entry from the state file that could not be restored, kept so the panel
    /// can explain why it opened without a root instead of silently dropping it.
    discarded: Option<String>,
}

/// Why the last root could not be recorded.
#[derive(Debug)]
pub enum LastRootError {
    /// The selected path does not exist or is not a directory.
    InvalidPath,
    /// The state file could not be written. The in-memory value is still valid
    /// for the current session, but the next launch will not see it.
    Write(std::io::Error),
}

impl std::fmt::Display for LastRootError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPath => formatter.write_str("路径不存在或不是目录"),
            Self::Write(error) => write!(formatter, "写入状态文件失败（{error}）"),
        }
    }
}

impl LastRoot {
    /// Load the remembered root for the current user.
    #[must_use]
    pub fn load() -> Self {
        Self::load_from(state_file(LAST_ROOT_FILE))
    }

    /// Remembered root, canonicalised, when it still is a directory.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Stored entry that was dropped on load because it is no longer usable.
    #[must_use]
    pub fn discarded(&self) -> Option<&str> {
        self.discarded.as_deref()
    }

    /// Empty state without a storage file, for tests in other modules.
    #[cfg(test)]
    #[must_use]
    pub fn empty_for_tests() -> Self {
        Self {
            storage: None,
            path: None,
            discarded: None,
        }
    }

    /// Remember `path` for the next launch.
    ///
    /// Returns whether the remembered value changed. An invalid path keeps the
    /// previous value and reports [`LastRootError::InvalidPath`]. A failed write
    /// reports [`LastRootError::Write`] but keeps the in-memory value, so the
    /// current session still behaves as if the root were remembered.
    pub fn record(&mut self, path: &Path) -> Result<bool, LastRootError> {
        let path = std::fs::canonicalize(path).map_err(|_| LastRootError::InvalidPath)?;
        if !path.is_dir() {
            return Err(LastRootError::InvalidPath);
        }
        let changed = self.path.as_deref() != Some(path.as_path());
        self.path = Some(path);
        self.discarded = None;
        self.save().map_err(LastRootError::Write)?;
        Ok(changed)
    }

    fn load_from(storage: Option<PathBuf>) -> Self {
        let mut path = None;
        let mut discarded = None;
        if let Some(line) = storage
            .as_ref()
            .and_then(|storage| read_state_text(storage))
            .map(|text| text.trim().to_string())
            .filter(|line| !line.is_empty())
        {
            match std::fs::canonicalize(&line) {
                Ok(candidate) if candidate.is_dir() => path = Some(candidate),
                _ => discarded = Some(line),
            }
        }
        Self {
            storage,
            path,
            discarded,
        }
    }

    fn save(&self) -> std::io::Result<()> {
        let Some(storage) = &self.storage else {
            return Ok(());
        };
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = storage.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(storage, format!("{}\n", path.to_string_lossy()))
    }
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
            .and_then(|path| read_state_text(path))
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

/// Persistent display preferences of the panel.
///
/// The theme choice and the language preference. Unlike the server
/// configuration these are local display preferences: they are not part of
/// `ServerConfig`, never affect the TFTP server and are stored in their own
/// small state file so the setup flag and the remembered root keep their
/// meaning. The theme applies immediately; the language preference is resolved
/// at the next start, because the running process renders its log and native
/// dialog titles only once.
pub struct UiPrefs {
    storage: Option<PathBuf>,
    theme_mode: ThemeMode,
    language_preference: LanguagePreference,
}

impl UiPrefs {
    /// Load the preferences for the current user.
    #[must_use]
    pub fn load() -> Self {
        Self::load_from(state_file(UI_PREFS_FILE))
    }

    /// The persisted theme choice (follow the system when unset or invalid).
    #[must_use]
    pub fn theme_mode(&self) -> ThemeMode {
        self.theme_mode
    }

    /// The persisted language preference (follow the system when unset or
    /// invalid).
    #[must_use]
    pub fn language_preference(&self) -> LanguagePreference {
        self.language_preference
    }

    /// Preferences without a storage file, for tests in other modules.
    #[cfg(test)]
    #[must_use]
    pub fn empty_for_tests() -> Self {
        Self {
            storage: None,
            theme_mode: ThemeMode::System,
            language_preference: LanguagePreference::System,
        }
    }

    /// Remember `mode` and write the state file.
    ///
    /// The in-memory value is updated even when the write fails, so the choice
    /// still applies for the current session; the caller decides whether to tell
    /// the operator.
    ///
    /// # Errors
    ///
    /// Returns the filesystem error when the state file cannot be written.
    pub fn set_theme_mode(&mut self, mode: ThemeMode) -> std::io::Result<()> {
        self.theme_mode = mode;
        self.save()
    }

    /// Remember the language preference for the next start and write the state
    /// file. The running panel is not re-rendered; the caller reports whether
    /// the preference could be stored.
    ///
    /// # Errors
    ///
    /// Returns the filesystem error when the state file cannot be written.
    pub fn set_language_preference(
        &mut self,
        preference: LanguagePreference,
    ) -> std::io::Result<()> {
        self.language_preference = preference;
        self.save()
    }

    fn load_from(storage: Option<PathBuf>) -> Self {
        let mut theme_mode = ThemeMode::System;
        let mut language_preference = LanguagePreference::System;
        if let Some(text) = storage
            .as_ref()
            .and_then(|storage| read_state_text(storage))
        {
            for line in text.lines() {
                let line = line.trim();
                if let Some(value) = line.strip_prefix("theme-mode=") {
                    if let Some(mode) = ThemeMode::parse(value) {
                        theme_mode = mode;
                    }
                } else if let Some(value) = line.strip_prefix("language=") {
                    if let Some(preference) = LanguagePreference::parse(value) {
                        language_preference = preference;
                    }
                }
            }
        }
        Self {
            storage,
            theme_mode,
            language_preference,
        }
    }

    fn save(&self) -> std::io::Result<()> {
        let Some(storage) = &self.storage else {
            return Ok(());
        };
        if let Some(parent) = storage.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(
            storage,
            format!(
                "theme-mode={}\nlanguage={}\n",
                self.theme_mode.as_str(),
                self.language_preference.as_str()
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_path(dir: &std::path::Path) -> PathBuf {
        dir.join(SETUP_STATE_FILE)
    }

    fn last_root_path(dir: &std::path::Path) -> PathBuf {
        dir.join(LAST_ROOT_FILE)
    }

    #[test]
    fn the_first_writable_state_directory_is_used() {
        let temp = tempfile::tempdir().expect("temp dir");
        let primary = temp.path().join("primary");
        let fallback = temp.path().join("fallback");
        let location = select_state_dir(vec![(primary.clone(), false), (fallback.clone(), true)])
            .expect("a writable directory");
        assert_eq!(location.dir, primary);
        assert!(!location.portable);
        assert!(primary.is_dir());
        assert!(!fallback.exists());
    }

    #[test]
    fn a_blocked_state_directory_falls_back_to_the_portable_one() {
        let temp = tempfile::tempdir().expect("temp dir");
        let blocked = temp.path().join("blocked");
        std::fs::write(&blocked, b"x").expect("write blocker");
        let fallback = temp.path().join("fallback");
        let location = select_state_dir(vec![(blocked, false), (fallback.clone(), true)])
            .expect("fallback directory");
        assert_eq!(location.dir, fallback);
        assert!(location.portable);
        assert!(fallback.is_dir());
    }

    #[test]
    fn missing_last_root_file_means_no_root_is_remembered() {
        let temp = tempfile::tempdir().expect("temp dir");
        let last = LastRoot::load_from(Some(last_root_path(temp.path())));
        assert_eq!(last.path(), None);
    }

    #[test]
    fn recording_a_directory_survives_a_reload() {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage = last_root_path(temp.path());
        let root = temp.path().join("share");
        std::fs::create_dir(&root).expect("root dir");

        let mut last = LastRoot::load_from(Some(storage.clone()));
        assert!(matches!(last.record(&root), Ok(true)));
        let expected = std::fs::canonicalize(&root).expect("canonical");
        assert_eq!(last.path(), Some(expected.as_path()));
        assert_eq!(last.discarded(), None);
        assert!(storage.is_file());
        // Recording the same directory again must not report a change.
        assert!(matches!(last.record(&root), Ok(false)));

        let reloaded = LastRoot::load_from(Some(storage));
        assert_eq!(reloaded.path(), Some(expected.as_path()));
        assert_eq!(reloaded.discarded(), None);
    }

    #[test]
    fn missing_or_non_directory_paths_are_not_remembered() {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage = last_root_path(temp.path());
        let file = temp.path().join("file.txt");
        std::fs::write(&file, b"x").expect("write file");

        let mut last = LastRoot::load_from(Some(storage.clone()));
        assert!(matches!(
            last.record(&temp.path().join("does-not-exist")),
            Err(LastRootError::InvalidPath)
        ));
        assert!(matches!(
            last.record(&file),
            Err(LastRootError::InvalidPath)
        ));
        assert_eq!(last.path(), None);
        assert!(!storage.exists());
    }

    #[test]
    fn a_stale_remembered_root_is_dropped_on_load() {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage = last_root_path(temp.path());
        let gone = temp.path().join("gone");
        std::fs::create_dir(&gone).expect("gone dir");
        std::fs::write(&storage, format!("{}\n", gone.display())).expect("write state");
        std::fs::remove_dir(&gone).expect("remove dir");

        let last = LastRoot::load_from(Some(storage));
        assert_eq!(last.path(), None);
        assert_eq!(
            last.discarded(),
            Some(gone.to_string_lossy().as_ref()),
            "the unusable entry is kept so the panel can explain it"
        );
    }

    #[test]
    fn an_invalid_record_keeps_the_previous_value() {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage = last_root_path(temp.path());
        let root = temp.path().join("share");
        std::fs::create_dir(&root).expect("root dir");
        let mut last = LastRoot::load_from(Some(storage));
        assert!(last.record(&root).is_ok());
        let remembered = last.path().map(Path::to_path_buf);
        assert!(last.record(&temp.path().join("missing")).is_err());
        assert_eq!(last.path().map(Path::to_path_buf), remembered);
    }

    #[test]
    fn a_read_only_profile_does_not_panic_when_recording_the_last_root() {
        let temp = tempfile::tempdir().expect("temp dir");
        let parent_file = temp.path().join("not-a-directory");
        std::fs::write(&parent_file, b"x").expect("write parent file");
        let mut last = LastRoot::load_from(Some(parent_file.join(LAST_ROOT_FILE)));
        // The path itself is a directory, but the profile path cannot be
        // created; the in-memory value must still be available for this run.
        assert!(matches!(
            last.record(temp.path()),
            Err(LastRootError::Write(_))
        ));
        assert_eq!(
            last.path(),
            Some(
                std::fs::canonicalize(temp.path())
                    .expect("canonical")
                    .as_path()
            )
        );
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

    #[test]
    fn the_theme_preference_round_trips_through_the_state_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage = temp.path().join(UI_PREFS_FILE);
        let mut prefs = UiPrefs::load_from(Some(storage.clone()));
        // Follow the system when nothing has been stored yet.
        assert_eq!(prefs.theme_mode(), ThemeMode::System);
        assert_eq!(prefs.language_preference(), LanguagePreference::System);

        prefs
            .set_theme_mode(ThemeMode::Dark)
            .expect("write the preference");
        assert_eq!(prefs.theme_mode(), ThemeMode::Dark);
        assert_eq!(
            std::fs::read_to_string(&storage).expect("read preference"),
            "theme-mode=dark\nlanguage=system\n"
        );
        assert_eq!(
            UiPrefs::load_from(Some(storage.clone())).theme_mode(),
            ThemeMode::Dark
        );

        prefs
            .set_theme_mode(ThemeMode::Light)
            .expect("overwrite the preference");
        assert_eq!(
            UiPrefs::load_from(Some(storage)).theme_mode(),
            ThemeMode::Light
        );

        // A file written before the language preference existed contains only
        // the theme key; it must keep working and default to "system".
        let legacy = temp.path().join("legacy-prefs.txt");
        std::fs::write(&legacy, "theme-mode=dark\n").expect("write legacy preference");
        let legacy = UiPrefs::load_from(Some(legacy));
        assert_eq!(legacy.theme_mode(), ThemeMode::Dark);
        assert_eq!(legacy.language_preference(), LanguagePreference::System);
    }

    #[test]
    fn the_language_preference_round_trips_through_the_state_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage = temp.path().join(UI_PREFS_FILE);
        let mut prefs = UiPrefs::load_from(Some(storage.clone()));
        assert_eq!(prefs.language_preference(), LanguagePreference::System);

        prefs
            .set_language_preference(LanguagePreference::ZhCn)
            .expect("write the preference");
        assert_eq!(prefs.language_preference(), LanguagePreference::ZhCn);
        assert_eq!(
            std::fs::read_to_string(&storage).expect("read preference"),
            "theme-mode=system\nlanguage=zh-CN\n"
        );
        assert_eq!(
            UiPrefs::load_from(Some(storage.clone())).language_preference(),
            LanguagePreference::ZhCn
        );

        // The two keys are independent and survive each other's writes.
        prefs
            .set_theme_mode(ThemeMode::Dark)
            .expect("write the theme");
        let reloaded = UiPrefs::load_from(Some(storage));
        assert_eq!(reloaded.theme_mode(), ThemeMode::Dark);
        assert_eq!(reloaded.language_preference(), LanguagePreference::ZhCn);
    }

    #[test]
    fn a_missing_or_unreadable_theme_preference_falls_back_to_the_system() {
        let temp = tempfile::tempdir().expect("temp dir");
        assert_eq!(
            UiPrefs::load_from(Some(temp.path().join(UI_PREFS_FILE))).theme_mode(),
            ThemeMode::System
        );

        let corrupt = temp.path().join("corrupt.txt");
        std::fs::write(
            &corrupt,
            "theme-mode=neon\nlanguage=klingon\nsomething-else=1\n",
        )
        .expect("write corrupt");
        let corrupt = UiPrefs::load_from(Some(corrupt));
        assert_eq!(corrupt.theme_mode(), ThemeMode::System);
        assert_eq!(corrupt.language_preference(), LanguagePreference::System);

        // A valid key next to a corrupt one is still used; only the corrupt key
        // falls back to its default.
        let partial = temp.path().join("partial.txt");
        std::fs::write(&partial, "theme-mode=dark\nlanguage=klingon\n")
            .expect("write partial preference");
        let partial = UiPrefs::load_from(Some(partial));
        assert_eq!(partial.theme_mode(), ThemeMode::Dark);
        assert_eq!(partial.language_preference(), LanguagePreference::System);

        // A directory where a file is expected is treated the same way.
        let directory = temp.path().join("prefs-dir");
        std::fs::create_dir(&directory).expect("prefs dir");
        let directory = UiPrefs::load_from(Some(directory));
        assert_eq!(directory.theme_mode(), ThemeMode::System);
        assert_eq!(directory.language_preference(), LanguagePreference::System);
    }

    #[test]
    fn a_failed_theme_write_keeps_the_choice_for_this_session() {
        let temp = tempfile::tempdir().expect("temp dir");
        let parent_file = temp.path().join("not-a-directory");
        std::fs::write(&parent_file, b"x").expect("write parent file");
        let mut prefs = UiPrefs::load_from(Some(parent_file.join(UI_PREFS_FILE)));
        // The write fails, but the operator still gets the chosen theme now.
        assert!(prefs.set_theme_mode(ThemeMode::Dark).is_err());
        assert_eq!(prefs.theme_mode(), ThemeMode::Dark);
    }

    #[test]
    fn a_failed_language_write_is_reported_and_keeps_the_selection() {
        let temp = tempfile::tempdir().expect("temp dir");
        let parent_file = temp.path().join("not-a-directory");
        std::fs::write(&parent_file, b"x").expect("write parent file");
        let mut prefs = UiPrefs::load_from(Some(parent_file.join(UI_PREFS_FILE)));
        // The running panel does not switch anyway; the error is what tells the
        // operator that the next start will keep the previous language.
        assert!(prefs
            .set_language_preference(LanguagePreference::ZhCn)
            .is_err());
        assert_eq!(prefs.language_preference(), LanguagePreference::ZhCn);
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_stored_values() {
        // An editor on Windows can add a UTF-8 BOM; without stripping it the
        // first key would never match and the value would be silently ignored.
        let temp = tempfile::tempdir().expect("temp dir");
        let theme = temp.path().join(UI_PREFS_FILE);
        std::fs::write(&theme, "\u{feff}theme-mode=dark\nlanguage=zh-CN\n").expect("write theme");
        let theme = UiPrefs::load_from(Some(theme));
        assert_eq!(theme.theme_mode(), ThemeMode::Dark);
        assert_eq!(theme.language_preference(), LanguagePreference::ZhCn);

        let setup = temp.path().join(SETUP_STATE_FILE);
        std::fs::write(&setup, "\u{feff}setup-shown=true\n").expect("write setup");
        assert!(!SetupState::load_from(Some(setup)).should_show_setup());

        let root = temp.path().join("share");
        std::fs::create_dir(&root).expect("share dir");
        let canonical = std::fs::canonicalize(&root).expect("canonical");
        let last_root = temp.path().join(LAST_ROOT_FILE);
        std::fs::write(
            &last_root,
            format!("\u{feff}{}\n", canonical.to_string_lossy()),
        )
        .expect("write last root");
        assert_eq!(
            LastRoot::load_from(Some(last_root)).path(),
            Some(canonical.as_path())
        );
    }
}

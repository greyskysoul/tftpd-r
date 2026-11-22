//! Theme preference and system dark-mode detection.
//!
//! The panel has a light and a dark palette (the Slint `Theme` global holds
//! both). The operator chooses between *follow the system*, *light* and *dark*;
//! the choice is persisted per user. In *follow the system* mode the panel has
//! to know the operating system's preference, which is why this module reads it
//! through the platform's own runtime:
//!
//! - Windows: `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`
//!   → `AppsUseLightTheme` (`0` = dark, `1` = light), read with `reg.exe`. The
//!   workspace forbids `unsafe`, so the registry is not opened through FFI.
//! - Unix: `gsettings get org.gnome.desktop.interface color-scheme`
//!   (`'prefer-dark'`), falling back to a `GTK_THEME` value ending in `dark`.
//!
//! Both helpers are fixed absolute programs with fixed arguments, no shell and
//! no user input. Detection is best effort: an unsupported platform, a missing
//! helper or an unparsable answer means "light", which is the palette the panel
//! shipped with before dark mode existed. A background watcher re-reads the
//! value so the panel follows a change of the system theme while it runs.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Interval between two system-theme probes while the panel follows the system.
///
/// The probe starts a helper process (~40 ms on Windows), so it must not run on
/// the UI thread and must not run more often than necessary. A changed system
/// theme is therefore applied with at most this much delay.
const POLL_INTERVAL: Duration = Duration::from_secs(30);

/// How long a platform helper may take before it is killed.
const HELPER_TIMEOUT: Duration = Duration::from_secs(3);

/// Windows `CREATE_NO_WINDOW`; the probe must not flash a console window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Which palette the panel uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    /// Follow the operating system's light/dark preference.
    #[default]
    System,
    /// Always use the light palette.
    Light,
    /// Always use the dark palette.
    Dark,
}

impl ThemeMode {
    /// Persisted spelling in `ui-prefs.txt`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// Parse the persisted spelling; unknown values are rejected so a corrupt
    /// file falls back to the default instead of silently picking a mode.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

    /// Value of the Slint `theme-mode` property (0 system, 1 light, 2 dark).
    #[must_use]
    pub fn slint_value(self) -> i32 {
        match self {
            Self::System => 0,
            Self::Light => 1,
            Self::Dark => 2,
        }
    }

    /// Inverse of [`Self::slint_value`]; unknown values are rejected.
    #[must_use]
    pub fn from_slint_value(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::System),
            1 => Some(Self::Light),
            2 => Some(Self::Dark),
            _ => None,
        }
    }

    /// Whether the panel is dark with this preference and `system_dark`.
    #[must_use]
    pub fn is_dark(self, system_dark: bool) -> bool {
        match self {
            Self::System => system_dark,
            Self::Light => false,
            Self::Dark => true,
        }
    }
}

/// Background probe of the system theme.
///
/// The first value is read synchronously in [`SystemThemeWatcher::start`] so
/// the window can be created with the right palette; later values are read by a
/// worker thread every [`POLL_INTERVAL`] and collected by the UI timer. The
/// probe only runs while [`Self::set_enabled`] is `true`, so an explicit
/// light/dark choice does not pay for it.
pub struct SystemThemeWatcher {
    latest: Arc<Mutex<Option<bool>>>,
    enabled: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl SystemThemeWatcher {
    /// Read the current system theme and start following changes.
    #[must_use]
    pub fn start() -> Self {
        let initial = system_is_dark();
        let latest = Arc::new(Mutex::new(Some(initial)));
        let enabled = Arc::new(AtomicBool::new(true));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let latest = Arc::clone(&latest);
            let enabled = Arc::clone(&enabled);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !sleep_unless_stopped(&stop, POLL_INTERVAL) {
                    if !enabled.load(Ordering::Relaxed) {
                        continue;
                    }
                    let value = platform_system_is_dark().unwrap_or(false);
                    if let Ok(mut guard) = latest.lock() {
                        *guard = Some(value);
                    }
                }
            })
        };
        Self {
            latest,
            enabled,
            stop,
            worker: Some(worker),
        }
    }

    /// The newest observed value. The first read always yields one, so this is
    /// `Some` from the moment the watcher exists.
    #[must_use]
    pub fn latest(&self) -> Option<bool> {
        self.latest.lock().ok().and_then(|guard| *guard)
    }

    /// Enable or disable the periodic probe.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    /// Ask the worker to stop. Idempotent; the worker is detached, so this never
    /// blocks shutdown.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take();
    }
}

impl Drop for SystemThemeWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Whether the operating system currently prefers a dark application theme.
///
/// Returns `false` (light) when the platform cannot be asked. This starts a
/// helper process and must not run on the UI thread while the panel is
/// interactive; use [`SystemThemeWatcher`] for that.
#[must_use]
pub fn system_is_dark() -> bool {
    platform_system_is_dark().unwrap_or(false)
}

/// Sleep for `duration`, waking early when `stop` is set.
///
/// Returns `true` when the watcher was asked to stop.
fn sleep_unless_stopped(stop: &AtomicBool, duration: Duration) -> bool {
    let step = Duration::from_millis(100);
    let mut slept = Duration::ZERO;
    while slept < duration {
        if stop.load(Ordering::Relaxed) {
            return true;
        }
        std::thread::sleep(step);
        slept += step;
    }
    stop.load(Ordering::Relaxed)
}

/// Parse `reg query … /v AppsUseLightTheme` output into "system is dark".
///
/// The expected line is `    AppsUseLightTheme    REG_DWORD    0x1`. Anything
/// else (missing value, unexpected type, non-hex number) is unparsable.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_windows_theme(stdout: &str) -> Option<bool> {
    for line in stdout.lines() {
        let mut parts = line.split_whitespace();
        if parts.next() != Some("AppsUseLightTheme") {
            continue;
        }
        if !parts.next()?.eq_ignore_ascii_case("REG_DWORD") {
            continue;
        }
        let value = parts.next()?;
        let value = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
            .unwrap_or(value);
        let light = u32::from_str_radix(value, 16).ok()?;
        return Some(light == 0);
    }
    None
}

/// Parse `gsettings get org.gnome.desktop.interface color-scheme` output.
///
/// Returns `None` when the value does not express a preference; GNOME uses
/// `'default'` for "no preference" and `'prefer-dark'`/`'prefer-light'` for an
/// explicit one. Other desktops often answer with the schema default or an
/// error, both of which are unparsable.
#[cfg_attr(windows, allow(dead_code))]
fn parse_gsettings_theme(stdout: &str) -> Option<bool> {
    let value = stdout.trim().trim_matches('\'').trim_matches('"');
    match value {
        "prefer-dark" => Some(true),
        "prefer-light" => Some(false),
        _ => None,
    }
}

/// `GTK_THEME=Adwaita:dark` is the only hint on desktops without gsettings.
#[cfg_attr(windows, allow(dead_code))]
fn parse_gtk_theme(theme: &str) -> Option<bool> {
    let theme = theme.trim();
    if theme.is_empty() {
        return None;
    }
    let name = theme.split(':').next_back().unwrap_or(theme);
    if name.to_ascii_lowercase().ends_with("dark") {
        Some(true)
    } else {
        Some(false)
    }
}

#[cfg(windows)]
fn platform_system_is_dark() -> Option<bool> {
    let program = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .map(|root| root.join("System32").join("reg.exe"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| std::path::PathBuf::from("reg.exe"));
    let mut command = Command::new(program);
    command.args([
        "query",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        "/v",
        "AppsUseLightTheme",
    ]);
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let stdout = run_with_timeout(command)?;
    parse_windows_theme(&String::from_utf8_lossy(&stdout))
}

#[cfg(not(windows))]
fn platform_system_is_dark() -> Option<bool> {
    if let Some(program) = ["/usr/bin/gsettings", "/bin/gsettings"]
        .into_iter()
        .find(|candidate| std::path::Path::new(candidate).is_file())
    {
        let mut command = Command::new(program);
        command.args(["get", "org.gnome.desktop.interface", "color-scheme"]);
        if let Some(dark) = run_with_timeout(command)
            .and_then(|stdout| parse_gsettings_theme(&String::from_utf8_lossy(&stdout)))
        {
            return Some(dark);
        }
    }
    std::env::var("GTK_THEME")
        .ok()
        .and_then(|theme| parse_gtk_theme(&theme))
}

/// Run a helper with a bounded run time and return its stdout.
fn run_with_timeout(mut command: Command) -> Option<Vec<u8>> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + HELPER_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok().map(|output| output.stdout),
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_roundtrip_through_their_persisted_and_slint_forms() {
        for mode in [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark] {
            assert_eq!(ThemeMode::parse(mode.as_str()), Some(mode));
            assert_eq!(ThemeMode::from_slint_value(mode.slint_value()), Some(mode));
        }
        assert_eq!(ThemeMode::default(), ThemeMode::System);
    }

    #[test]
    fn unknown_persisted_values_are_rejected() {
        assert_eq!(ThemeMode::parse(""), None);
        assert_eq!(ThemeMode::parse("Dark"), None);
        assert_eq!(ThemeMode::parse("auto"), None);
        assert_eq!(ThemeMode::from_slint_value(3), None);
        assert_eq!(ThemeMode::from_slint_value(-1), None);
    }

    #[test]
    fn the_system_mode_follows_the_system_value() {
        assert!(ThemeMode::System.is_dark(true));
        assert!(!ThemeMode::System.is_dark(false));
        assert!(!ThemeMode::Light.is_dark(true));
        assert!(ThemeMode::Dark.is_dark(false));
    }

    #[test]
    fn the_windows_registry_answer_is_parsed() {
        let light = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\r\n    AppsUseLightTheme    REG_DWORD    0x1\r\n\r\n";
        assert_eq!(parse_windows_theme(light), Some(false));
        let dark = "    AppsUseLightTheme    REG_DWORD    0x0";
        assert_eq!(parse_windows_theme(dark), Some(true));
    }

    #[test]
    fn an_unparsable_registry_answer_is_ignored() {
        assert_eq!(parse_windows_theme(""), None);
        assert_eq!(
            parse_windows_theme(
                "ERROR: The system was unable to find the specified registry key or value."
            ),
            None
        );
        assert_eq!(
            parse_windows_theme("    AppsUseLightTheme    REG_SZ    yes"),
            None
        );
        assert_eq!(
            parse_windows_theme("    AppsUseLightTheme    REG_DWORD    nope"),
            None
        );
    }

    #[test]
    fn the_gsettings_answer_is_parsed() {
        assert_eq!(parse_gsettings_theme("'prefer-dark'\n"), Some(true));
        assert_eq!(parse_gsettings_theme("'prefer-light'\n"), Some(false));
        assert_eq!(parse_gsettings_theme("'default'\n"), None);
        assert_eq!(parse_gsettings_theme(""), None);
    }

    #[test]
    fn the_gtk_theme_variable_is_used_as_a_fallback() {
        assert_eq!(parse_gtk_theme("Adwaita:dark"), Some(true));
        assert_eq!(parse_gtk_theme("Adwaita:light"), Some(false));
        assert_eq!(parse_gtk_theme("Adwaita"), Some(false));
        assert_eq!(parse_gtk_theme(""), None);
    }

    #[test]
    fn the_watcher_exposes_the_initial_value_and_stops_cleanly() {
        let mut watcher = SystemThemeWatcher::start();
        // The initial read always produces a value, so the panel can build its
        // first frame deterministically.
        assert!(watcher.latest().is_some());
        watcher.set_enabled(false);
        watcher.stop();
        // Stopping twice is harmless (the panel stops it on shutdown and the
        // drop impl repeats the request).
        watcher.stop();
    }
}

//! Slint desktop panel for the TFTP server (embedded mode).
//!
//! The GUI runs the `tftpd-r-core` server in its own Tokio runtime, polls the
//! bounded event bus from a Slint timer, and never blocks the UI thread while
//! stopping or restarting the server.

// The panel is a windowed application; a console window must not be allocated
// on start. `--help` output is only visible when stdout is redirected.
#![cfg_attr(windows, windows_subsystem = "windows")]

slint::include_modules!();

mod controller;
mod folder_picker;
mod i18n;
mod log_export;
mod logging;
mod netif;
mod recent;
mod single_instance;
mod theme;
mod user_state;

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use chrono::Local;
use slint::{CloseRequestResponse, Model, ModelRc, Timer, TimerMode, VecModel};
use tftpd_r_core::config::display_path;
use tftpd_r_core::events::{EventBus, EventKind, SessionId, TftpEvent};
use tftpd_r_core::server::ServerStatus;
use tftpd_r_core::ServerConfig;

use controller::EmbeddedController;
use folder_picker::FolderPicker;
use i18n::{Language, LanguagePreference, Text};
use log_export::SaveChoice;
use logging::{
    export_file_name, format_export, write_export, LogEntry, LogFilter, LogLevel, MAX_LOG_ROWS,
};
use recent::RecentDirs;
use single_instance::{ActivationRequest, Background, Owner};
use theme::{SystemThemeWatcher, ThemeMode};
use user_state::{fallback_state_dir, LastRoot, SetupState, UiPrefs};

const UI_TICK: Duration = Duration::from_millis(200);

/// Product icon pixels for the window and task-bar icon, generated from
/// `ui/icons/app-icon.svg` (see `ui/icons/README.md`). The panel build has no
/// runtime image decoder, so the pixels are compiled in and uploaded as a raw
/// buffer instead of going through `@image-url`.
const APP_ICON_RGBA: &[u8] = include_bytes!("../ui/icons/app-icon-128.rgba");
/// Edge length of `APP_ICON_RGBA`; Slint asks winit for 64 px logical, so 128 px
/// stays sharp up to 200 % display scaling.
const APP_ICON_SIZE: u32 = 128;

/// Wrap the embedded product icon in a Slint image for `Window.icon`.
fn app_icon() -> slint::Image {
    let mut buffer =
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(APP_ICON_SIZE, APP_ICON_SIZE);
    // The asset length is a multiple of four bytes (asserted by a unit test), so
    // the remainder returned here is always empty.
    let (pixels, _) = APP_ICON_RGBA.as_chunks::<4>();
    for (pixel, rgba) in buffer.make_mut_slice().iter_mut().zip(pixels) {
        *pixel = slint::Rgba8Pixel::new(rgba[0], rgba[1], rgba[2], rgba[3]);
    }
    slint::Image::from_rgba8(buffer)
}

/// Command that hands `url` to the platform's default browser without a shell.
///
/// Split from [`open_repository_url`] so tests can assert the exact program and
/// argument vector. `url` is always the compile-time Cargo repository metadata,
/// never user input.
fn repository_command(url: &str) -> Command {
    #[cfg(target_os = "windows")]
    {
        // `FileProtocolHandler` resolves the default browser; using
        // `cmd /c start` would introduce a shell.
        let system_root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let mut command = Command::new(system_root.join("System32").join("rundll32.exe"));
        command.arg("url.dll,FileProtocolHandler").arg(url);
        command
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Desktop Linux provides `xdg-open`. A missing helper is reported to
        // the activity log instead of crashing the panel.
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    }
}

/// Open the public project page in the default browser.
fn open_repository_url() -> std::io::Result<()> {
    repository_command(env!("CARGO_PKG_REPOSITORY"))
        .spawn()
        .map(|_| ())
}

struct TransferState {
    file: String,
    peer: String,
    bytes: u64,
    total: Option<u64>,
    started: Instant,
}

/// One entry of the bind address picker, kept in the UI state so selections
/// survive a discovery refresh.
struct AddressChoice {
    ip: IpAddr,
    label: String,
    selected: bool,
}

/// State of the "TftpdR is already running" prompt of a second launch.
///
/// Present only while the operator has not decided yet. The prompt is shown
/// before the panel is visible, so the second process never flashes a full
/// window for a launch that is about to exit.
struct SingleInstancePrompt {
    /// The panel that was already running when this one started.
    background: Background,
    /// Activation request sent by "Show program window", kept until the timer sees
    /// the answer (or the request expires).
    request: Option<ActivationRequest>,
}

struct UiState {
    last_seq: u64,
    last_status_key: i32,
    /// Localised operator-facing text for the language chosen at startup.
    text: Text,
    /// Theme choice of the operator; `None` would mean "unset", but the
    /// preference always resolves to [`ThemeMode::System`] by default.
    theme_mode: ThemeMode,
    /// Persisted UI preferences (currently only the theme choice).
    prefs: UiPrefs,
    /// Newest observed system dark-mode value, refreshed from the watcher.
    system_dark: bool,
    /// Retained activity-log entries, newest first.
    logs: VecDeque<LogEntry>,
    /// Which severity the log table shows (display-only filter).
    log_filter: LogFilter,
    /// Filtered rows bound to the Slint log table.
    log_model: Rc<VecModel<LogRow>>,
    /// The model must be rebuilt after new entries or a filter change.
    logs_dirty: bool,
    transfers_model: Rc<VecModel<TransferRow>>,
    transfers: HashMap<SessionId, TransferState>,
    recent: RecentDirs,
    recent_model: Rc<VecModel<RecentDir>>,
    /// Rows currently held by `recent_model`. Rebuilding the model resets the
    /// Slint repeater and recreates every entry, which drops its hover state and
    /// breaks a press that is still in progress, so the model is only replaced
    /// when these rows differ (see `refresh_recent_model`).
    recent_rows: Vec<RecentDir>,
    /// Root directory of the last successful start, restored on the next
    /// launch when no `config.toml` provides one.
    last_root: LastRoot,
    config: ServerConfig,
    /// Root directory of the server instance that is starting or running.
    running_root: Option<PathBuf>,
    /// Configuration of the running server, for detecting changed settings.
    running_config: Option<ServerConfig>,
    /// Message of the last failed start/restart, shown in the status bar.
    failed_message: Option<String>,
    throughput_sample: (Instant, u64),
    /// Bind addresses shown in the settings picker.
    address_options: Vec<AddressChoice>,
    /// Local interface addresses discovered on a background thread.
    discovered: Arc<StdMutex<Option<Vec<IpAddr>>>>,
    discovery_applied: bool,
    /// Background native folder picker; the helper is pre-warmed when a dialog
    /// opens so the click path only pays for the dialog itself.
    folder_picker: FolderPicker,
    /// Pending native folder dialog; polled from the UI timer.
    pending_pick: Option<PendingFolderPick>,
    /// Whether the warm picker helper was requested in this session.
    picker_prewarmed: bool,
    /// Pending log-export save dialog; polled from the UI timer.
    log_export: Option<PendingLogExport>,
    /// "Already running" prompt of a second launch; `None` once the operator
    /// decided to start this instance.
    single_instance_prompt: Option<SingleInstancePrompt>,
    /// Whether the "Close TftpdR?" choice is currently covering the panel.
    close_prompt_open: bool,
    /// Set after "Run in background"; the window is hidden on the next UI tick
    /// so the tray icon's visibility change is processed first.
    pending_background: bool,
}

/// Which part of the panel asked for the native folder dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FolderPickTarget {
    /// The root field of the settings dialog: update the form only.
    SettingsForm,
    /// The recent-root menu: apply the directory and restart when idle.
    Root,
}

/// A folder dialog running in a helper process.
struct PendingFolderPick {
    target: FolderPickTarget,
    /// Settings-form hint shown before the dialog opened; restored when the
    /// dialog is cancelled or the selected path is unusable.
    previous_hint: String,
    receiver: Receiver<Option<PathBuf>>,
}

/// Finished folder-dialog request, handed from [`poll_folder_picker`] to the
/// matching UI action.
struct FolderPickOutcome {
    target: FolderPickTarget,
    picked: Option<PathBuf>,
    previous_hint: String,
}

/// A save-file dialog plus the log text snapshot it will write.
struct PendingLogExport {
    receiver: Receiver<SaveChoice>,
    text: String,
}

/// Language and appearance settings of the panel, decided at startup.
///
/// Grouped so [`UiState::new`] stays readable and so the two things that are
/// *not* server configuration (the startup language and the theme preference)
/// travel together.
struct Appearance {
    text: Text,
    theme_mode: ThemeMode,
    prefs: UiPrefs,
    system_dark: bool,
}

impl Appearance {
    fn new(text: Text, theme_mode: ThemeMode, prefs: UiPrefs, system_dark: bool) -> Self {
        Self {
            text,
            theme_mode,
            prefs,
            system_dark,
        }
    }

    /// English panel that follows the system theme, without a state file.
    #[cfg(test)]
    fn for_tests(text: Text) -> Self {
        Self::new(text, ThemeMode::System, UiPrefs::empty_for_tests(), false)
    }
}

impl UiState {
    fn new(
        config: ServerConfig,
        recent: RecentDirs,
        last_root: LastRoot,
        discovered: Arc<StdMutex<Option<Vec<IpAddr>>>>,
        appearance: Appearance,
    ) -> Self {
        let Appearance {
            text,
            theme_mode,
            prefs,
            system_dark,
        } = appearance;
        Self {
            last_seq: 0,
            last_status_key: 0,
            text,
            theme_mode,
            prefs,
            system_dark,
            logs: VecDeque::new(),
            log_filter: LogFilter::All,
            log_model: Rc::new(VecModel::default()),
            logs_dirty: false,
            transfers_model: Rc::new(VecModel::default()),
            transfers: HashMap::new(),
            recent,
            recent_model: Rc::new(VecModel::default()),
            recent_rows: Vec::new(),
            last_root,
            config,
            running_root: None,
            running_config: None,
            failed_message: None,
            throughput_sample: (Instant::now(), 0),
            address_options: Vec::new(),
            discovered,
            discovery_applied: false,
            folder_picker: FolderPicker::new(text.folder_dialog_title().to_string()),
            pending_pick: None,
            picker_prewarmed: false,
            log_export: None,
            single_instance_prompt: None,
            close_prompt_open: false,
            pending_background: false,
        }
    }

    /// Whether the panel is currently dark.
    fn is_dark(&self) -> bool {
        self.theme_mode.is_dark(self.system_dark)
    }

    fn push_log(&mut self, level: LogLevel, message: impl Into<String>) {
        self.logs.push_front(LogEntry::now(level, message));
        self.logs.truncate(MAX_LOG_ROWS);
        self.logs_dirty = true;
    }

    /// Rebuild the filtered Slint rows after new entries or a filter change.
    fn rebuild_log_model(&mut self) {
        let rows: Vec<LogRow> = self
            .logs
            .iter()
            .filter(|entry| self.log_filter.allows(entry.level))
            .map(|entry| LogRow {
                time: entry.display_time().into(),
                level: entry.level.as_str().into(),
                message: entry.message.clone().into(),
            })
            .collect();
        self.log_model.set_vec(rows);
    }

    /// Rebuild the recent-root rows when they actually changed.
    ///
    /// `VecModel::set_vec` emits a model reset, which makes Slint destroy and
    /// recreate every repeated row. Doing that on each 200 ms tick clears the
    /// hover state of the row under the pointer and invalidates the press a
    /// click is made of, so an unchanged list must not be replaced. Returns
    /// whether the model was rebuilt (the caller then re-attaches it).
    fn refresh_recent_model(&mut self) -> bool {
        let active_root = self
            .running_root
            .clone()
            .unwrap_or_else(|| self.config.root_dir.clone());
        let active_root = if root_is_set(&active_root) {
            std::fs::canonicalize(&active_root).unwrap_or(active_root)
        } else {
            active_root
        };
        let rows: Vec<RecentDir> = self
            .recent
            .paths()
            .iter()
            .map(|path| RecentDir {
                path: display_path(path).into(),
                name: recent_name(path).into(),
                active: same_path(path, &active_root),
            })
            .collect();
        if rows == self.recent_rows {
            return false;
        }
        self.recent_rows = rows.clone();
        self.recent_model.set_vec(rows);
        true
    }

    /// Remember the selected root for the next launch.
    ///
    /// Only an existing directory is stored, so a later launch can preselect it
    /// instead of falling back to "未指定根目录". This is local UI state, not
    /// configuration: `config.toml` still wins when it exists.
    fn remember_root(&mut self) {
        if root_is_set(&self.config.root_dir) {
            let root = self.config.root_dir.clone();
            match self.last_root.record(&root) {
                Ok(true) => {
                    let message = self.text.root_remembered(&display_root_path(&root));
                    self.push_log(LogLevel::Info, message);
                }
                Ok(false) => {}
                Err(error) => {
                    let message = self.text.root_not_remembered(&error.to_string());
                    self.push_log(LogLevel::Warn, message);
                }
            }
        }
    }

    fn discovered_snapshot(&self) -> Option<Vec<IpAddr>> {
        self.discovered.lock().ok().and_then(|guard| guard.clone())
    }

    /// Take a newly discovered address list exactly once.
    fn apply_discovered(&mut self) -> bool {
        if self.discovery_applied {
            return false;
        }
        let Some(discovered) = self.discovered_snapshot() else {
            return false;
        };
        self.discovery_applied = true;
        let selected = self.selected_addresses();
        self.rebuild_address_options(&selected, &discovered);
        true
    }

    fn selected_addresses(&self) -> Vec<IpAddr> {
        self.address_options
            .iter()
            .filter(|option| option.selected)
            .map(|option| option.ip)
            .collect()
    }

    fn rebuild_address_options(&mut self, selected: &[IpAddr], discovered: &[IpAddr]) {
        self.address_options = build_address_choices(selected, discovered, self.text);
    }

    fn bind_summary(&self) -> String {
        bind_summary_text(&self.address_options, self.text)
    }
}

/// Build the picker entries: the two mandated special addresses first, then the
/// discovered interface addresses, then any selected address that discovery did
/// not report (so a hand-written configuration is never silently dropped).
fn build_address_choices(
    selected: &[IpAddr],
    discovered: &[IpAddr],
    text: Text,
) -> Vec<AddressChoice> {
    let mut ips: Vec<IpAddr> = vec![
        IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        IpAddr::V4(Ipv4Addr::LOCALHOST),
    ];
    for ip in discovered.iter().chain(selected.iter()) {
        if !ips.contains(ip) {
            ips.push(*ip);
        }
    }
    ips.into_iter()
        .map(|ip| AddressChoice {
            ip,
            label: address_label(ip, text),
            selected: selected.contains(&ip),
        })
        .collect()
}

fn bind_summary_text(options: &[AddressChoice], text: Text) -> String {
    let selected: Vec<&AddressChoice> = options.iter().filter(|option| option.selected).collect();
    match selected.len() {
        0 => text.no_address_selected().to_string(),
        1 | 2 => selected
            .iter()
            .map(|option| option.label.clone())
            .collect::<Vec<_>>()
            .join(text.list_separator()),
        count => text.address_count_summary(&selected[0].label, &selected[1].label, count),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|argument| argument == "--help" || argument == "-h") {
        println!("tftpd-r {}", env!("CARGO_PKG_VERSION"));
        println!();
        println!("TftpdR native Slint panel for the embedded TFTP server (read-only RRQ).");
        println!();
        println!("Usage: tftpd-r [OPTIONS]");
        println!();
        println!("Options:");
        println!("  -h, --help        Print this help and exit");
        println!(
            "      --smoke-test  Create the window, run a few UI ticks and exit (development)"
        );
        println!("      --activate-only  Ask a running panel to show its window, then exit");
        println!("      --new-instance   Start without the \"already running\" prompt");
        return Ok(());
    }
    let arguments: Vec<String> = std::env::args().collect();
    let smoke_test = arguments.iter().any(|argument| argument == "--smoke-test");
    let launch = launch_mode(&arguments);
    // A second launch asks the running panel to show its window and exits. This
    // path never creates a window and therefore runs before any UI setup; the
    // operator who double-clicked the executable gets the panel they already
    // have instead of a duplicate.
    if launch == LaunchMode::ActivateOnly {
        return activate_running_panel();
    }
    let initial = load_initial_config();
    let last_root = LastRoot::load();
    // Keep the unusable entry and the remembered path for the startup log
    // before `last_root` moves into the UI state.
    let stale_root = last_root.discarded().map(ToOwned::to_owned);
    let remembered_root = last_root.path().map(Path::to_path_buf);
    let initial_config = initial.config(last_root.path());
    // Without a `config.toml`, the root comes either from the previous start or
    // from no selection at all; log which one so the operator can tell.
    let restored_root = (!initial.is_configured() && root_is_set(&initial_config.root_dir))
        .then(|| display_root_path(&initial_config.root_dir));
    let mut recent = RecentDirs::load();
    // Make sure the configured root shows up in the drop-down even on first run
    // or when it differs from the last used directory. A panel without a root
    // directory has nothing to show and must not create a history entry.
    if root_is_set(&initial_config.root_dir) {
        recent.record(&initial_config.root_dir);
    }

    // Interface discovery starts a platform helper; it must not block the
    // window creation, so it runs on a background thread and the options are
    // merged in from the UI timer once the result is available.
    let discovered = Arc::new(StdMutex::new(None));
    spawn_discovery(discovered.clone());

    // Language: resolved once, before the window exists. The persisted
    // preference (system / English / Simplified Chinese) is the normal source;
    // Simplified Chinese is the only supported translation, and every other
    // locale (including a failed detection) keeps the English source strings.
    // The choice drives both the bundled Slint translation and the strings the
    // Rust side builds. Changing the preference in the settings dialog stores it
    // for the next start instead of re-rendering this process.
    //
    // `TFTPD_R_GUI_LANGUAGE` is a development/diagnostic override: a machine
    // whose Windows UI language is, say, English cannot otherwise be used to
    // look at the Chinese layout. It is not reachable from the UI and is
    // documented as a development aid only.
    let prefs = UiPrefs::load();
    let language_preference = prefs.language_preference();
    let language = match std::env::var("TFTPD_R_GUI_LANGUAGE") {
        Ok(value) => Language::from_locale(Some(&value)),
        Err(_) => language_preference.resolve(),
    };
    let text = Text::new(language);

    // Theme: the persisted preference decides between following the system and
    // forcing light/dark. Reading the system value starts a helper process, so
    // it happens once here (before the first frame) and afterwards on the
    // watcher thread.
    let theme_mode = prefs.theme_mode();
    let theme_watcher = SystemThemeWatcher::start();
    let system_dark = theme_watcher.latest().unwrap_or(false);
    theme_watcher.set_enabled(theme_mode == ThemeMode::System);
    // The watcher must outlive the UI timer closure (which polls it) and is
    // stopped when the event loop ends.
    let theme_watcher_slot: Rc<RefCell<Option<SystemThemeWatcher>>> =
        Rc::new(RefCell::new(Some(theme_watcher)));

    let app = AppWindow::new()?;
    app.set_app_icon(app_icon());
    app.set_app_version(env!("CARGO_PKG_VERSION").into());
    app.set_repository_url(env!("CARGO_PKG_REPOSITORY").into());
    app.set_language(language.display_name().into());
    app.set_language_mode(language_preference.slint_value());
    // Selecting the bundled catalogue must happen after the first component
    // exists; `"en"` selects the source strings. This also overrides Slint's
    // own locale guess, which would treat e.g. zh-TW as Chinese.
    let translation = slint::select_bundled_translation(language.slint_locale());
    if let Err(error) = &translation {
        eprintln!(
            "warning: cannot select the bundled translation {:?}: {error}",
            language.slint_locale()
        );
    }

    let mut state = UiState::new(
        initial_config,
        recent,
        last_root,
        discovered,
        Appearance::new(text, theme_mode, prefs, system_dark),
    );
    // Single instance: a launch that finds another panel must not silently
    // create a second one. The check is skipped for development runs
    // (`--smoke-test`, screenshot previews) and for `--new-instance`, where the
    // operator already said "start another panel". The owner is registered
    // immediately unless the prompt is shown; registering on every start keeps
    // the third launch's prompt working after "New instance" was chosen.
    let single_instance: Rc<RefCell<Option<Owner>>> = Rc::new(RefCell::new(None));
    // Development aid for UI screenshots and manual layout checks: open a dialog
    // at startup. Not reachable without the environment variable. It is also
    // what keeps preview runs out of the single-instance prompt.
    let preview = std::env::var("TFTPD_R_GUI_PREVIEW").ok();
    if launch == LaunchMode::Normal && preview.is_none() && !smoke_test {
        match check_launch(&mut state, &single_instance) {
            Ok(()) => {}
            Err(error) => {
                eprintln!("warning: {error}");
                state.push_log(LogLevel::Warn, error);
            }
        }
    }
    app.set_log_filter(state.log_filter.key().into());
    app.set_theme_mode(state.theme_mode.slint_value());
    apply_theme(&app, &state);
    apply_config_to_settings(&app, &mut state);

    // First start without a usable `config.toml`: open the settings dialog so
    // the operator can fill in the root directory and bind addresses before the
    // first start. The flag is per user, so this happens once; `--smoke-test` and
    // the screenshot previews are development runs and must not consume it, and
    // neither does a second launch whose operator is still choosing between the
    // running panel and a new instance.
    let mut setup = SetupState::load();
    let show_setup = preview.is_none()
        && !smoke_test
        && state.single_instance_prompt.is_none()
        && !initial.is_configured()
        && setup.should_show_setup();
    if show_setup {
        app.set_settings_open(true);
        setup.mark_setup_shown();
    }

    // Development aid for UI screenshots and manual layout checks: open a
    // dialog at startup. Not reachable without the environment variable.
    match preview.as_deref() {
        Some("settings") => {
            app.set_about_open(false);
            app.set_settings_open(true);
        }
        Some("about") => {
            app.set_settings_open(false);
            app.set_about_open(true);
        }
        Some("picker") => {
            app.set_about_open(false);
            app.set_settings_open(true);
            app.set_settings_picker_open(true);
        }
        Some("root-menu") => {
            app.set_settings_open(false);
            app.set_about_open(false);
            app.set_recent_menu_open(true);
        }
        Some("log-filter") => {
            app.set_settings_open(false);
            app.set_about_open(false);
            app.set_log_filter_open(true);
        }
        Some("transfer") => {
            app.set_settings_open(false);
            app.set_about_open(false);
            // Fake rows so the transfer table and progress fill can be
            // screenshotted without a client; never reachable without the
            // environment variable.
            state.transfers.insert(
                1,
                TransferState {
                    file: "firmware.bin".to_string(),
                    peer: "192.168.1.20:49152".to_string(),
                    bytes: 1_100_000,
                    total: Some(2_000_000),
                    started: Instant::now(),
                },
            );
            state.transfers.insert(
                2,
                TransferState {
                    file: "config.bin".to_string(),
                    peer: "127.0.0.1:50000".to_string(),
                    bytes: 900_000,
                    total: None,
                    started: Instant::now(),
                },
            );
        }
        _ => {}
    }
    state.refresh_recent_model();
    // Attach the (possibly non-empty) history once; later ticks only replace the
    // shared `VecModel` when the rows really change, so the menu keeps the hover
    // state and the press target of the row under the pointer.
    app.set_recent_dirs(ModelRc::from(state.recent_model.clone()));
    state.push_log(
        LogLevel::Info,
        state.text.panel_started(env!("CARGO_PKG_VERSION")),
    );
    if let Some(dir) = fallback_state_dir() {
        let message = state.text.portable_state_dir(&display_path(dir));
        state.push_log(LogLevel::Warn, message);
    }
    if let Some(root) = restored_root {
        let message = state.text.root_restored(&root);
        state.push_log(LogLevel::Info, message);
    }
    if let Some(stale) = stale_root {
        let message = state.text.root_stale(&display_path(Path::new(&stale)));
        state.push_log(LogLevel::Warn, message);
    }
    if initial.is_configured() {
        state.push_log(LogLevel::Info, state.text.config_loaded());
        if remembered_root.is_some() {
            state.push_log(LogLevel::Info, state.text.config_overrides_memory());
        }
    }
    if let Some(error) = initial.error() {
        let message = state.text.config_unusable(error);
        state.push_log(LogLevel::Warn, message);
    }
    if show_setup {
        state.push_log(LogLevel::Info, state.text.first_start());
    }
    let state = Rc::new(RefCell::new(state));
    let controller = Rc::new(RefCell::new(EmbeddedController::new()?));

    // Notification-area icon. The Slint tray item is created lazily on the
    // first "Run in background" choice, because its platform handle is created
    // from the first event-loop turn and would otherwise be registered (and
    // fail) at startup.
    let tray: Rc<RefCell<Option<TrayIcon>>> = Rc::new(RefCell::new(None));
    app.set_background_available(true);

    // The "already running" prompt of a second launch is its own small window.
    // The main panel exists but is not shown until the operator picks "New
    // instance", so an activation or cancel never flashes the full program.
    let prompt_window: Rc<RefCell<Option<InstancePromptWindow>>> = Rc::new(RefCell::new(None));
    let prompt_pid = state
        .borrow()
        .single_instance_prompt
        .as_ref()
        .map(|prompt| prompt.background.pid);
    if let Some(pid) = prompt_pid {
        let prompt = InstancePromptWindow::new()?;
        prompt.set_app_icon(app_icon());
        prompt.set_dark_mode(state.borrow().is_dark());
        prompt.set_pid(i32::try_from(pid).unwrap_or(0));
        install_instance_prompt_callbacks(&prompt, &app, &state, &single_instance, &prompt_window);
        prompt.show()?;
        *prompt_window.borrow_mut() = Some(prompt);
    }

    // A launch that has nothing to ask (or was started with `--new-instance`)
    // owns its lock immediately, so later launches can find and activate it.
    // When the prompt is open nothing is registered: "Cancel" and "Activate"
    // must exit without leaving a lock file behind, and "New instance" registers
    // from its callback.
    if state.borrow().single_instance_prompt.is_none() {
        register_single_instance(&mut state.borrow_mut(), &single_instance);
    }

    apply_status(&app, &ServerStatus::Stopped, &state.borrow());
    install_callbacks(&app, &controller, &state, &theme_watcher_slot, &tray);

    let timer = Timer::default();
    {
        let weak = app.as_weak();
        let controller = controller.clone();
        let state = state.clone();
        let theme_watcher = theme_watcher_slot.clone();
        let single_instance = single_instance.clone();
        let prompt_window = prompt_window.clone();
        let tray = tray.clone();
        timer.start(TimerMode::Repeated, UI_TICK, move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let events = controller.borrow().events();
            let status = controller.borrow_mut().poll_status();
            let restart_error = controller.borrow_mut().take_error();
            let system_dark = theme_watcher
                .borrow()
                .as_ref()
                .and_then(SystemThemeWatcher::latest)
                .unwrap_or(false);
            let mut state = state.borrow_mut();
            if service_single_instance(&app, &prompt_window, &mut state, &single_instance, &tray) {
                return;
            }
            // "Run in background" hides the window only after the tray icon's
            // visibility change had one event-loop turn to reach the platform.
            // If the tray handle could not be created, the hidden window makes
            // the backend's last-window rule stop the process instead of
            // leaving an unreachable background process.
            if state.pending_background {
                state.pending_background = false;
                if state.close_prompt_open {
                    // The operator reopened the close prompt in the meantime;
                    // put the icon away and let them choose again.
                    if let Some(tray_icon) = tray.borrow().as_ref() {
                        tray_icon.set_tray_visible(false);
                    }
                } else if let Err(error) = app.window().hide() {
                    if let Some(tray_icon) = tray.borrow().as_ref() {
                        tray_icon.set_tray_visible(false);
                    }
                    let message = state.text.window_hide_failed(&error.to_string());
                    state.push_log(LogLevel::Warn, message);
                } else {
                    let message = state.text.background_running();
                    state.push_log(LogLevel::Info, message);
                }
            }
            // A changed system scheme is applied on the next tick, at most one
            // probe interval after the operating system changed.
            if state.system_dark != system_dark {
                state.system_dark = system_dark;
                apply_theme(&app, &state);
                if let Some(prompt) = prompt_window.borrow().as_ref() {
                    prompt.set_dark_mode(state.is_dark());
                }
            }
            drain_events(&events, &mut state);
            if let Some(error) = restart_error {
                state.failed_message = Some(error.clone());
                let message = state.text.restart_failed(&error);
                state.push_log(LogLevel::Error, message);
            }
            if state.apply_discovered() {
                apply_address_options(&app, &state);
            }
            // Start the helper as soon as a dialog that contains a folder
            // picker opens. The first click then usually only waits for the
            // dialog itself (~0.2 s) instead of PowerShell startup (~1.6 s).
            // The recent-root menu is included because its 选择… button opens
            // the same picker without the settings dialog.
            if !state.picker_prewarmed && (app.get_settings_open() || app.get_recent_menu_open()) {
                state.picker_prewarmed = true;
                state.folder_picker.prewarm();
            }
            if let Some(outcome) = poll_folder_picker(&mut state) {
                match outcome.target {
                    FolderPickTarget::SettingsForm => {
                        handle_folder_pick(&app, &mut state, outcome.picked, &outcome.previous_hint)
                    }
                    FolderPickTarget::Root => {
                        handle_root_pick(&app, &controller, &mut state, outcome.picked)
                    }
                }
            }
            if let Some((choice, text)) = poll_log_export(&mut state) {
                finish_log_export(&app, &mut state, choice, text);
            }
            sync_running_root(&mut state, &status);
            log_status_transition(&mut state, &status);
            refresh_models(&app, &mut state);
            apply_status(&app, &status, &state);
            app.set_window_maximized(app.window().is_maximized());
        });
    }

    let quit_timer = Timer::default();
    if smoke_test {
        // Development smoke test: build the window, run a few UI ticks and exit.
        // Report the resolved language, the translated window title (proof that
        // the bundled catalogue was applied) and the resolved palette, so a
        // headless run shows the whole startup decision.
        println!(
            "smoke-test: language={} language_pref={} translation={} tagline={} theme={} dark={}",
            language.slint_locale(),
            language_preference.as_str(),
            if translation.is_ok() { "ok" } else { "missing" },
            app.get_tagline(),
            state.borrow().theme_mode.as_str(),
            state.borrow().is_dark(),
        );
        quit_timer.start(TimerMode::SingleShot, Duration::from_millis(1500), || {
            let _ = slint::quit_event_loop();
        });
    }

    // The second-launch prompt is already visible and the full panel must stay
    // hidden until the operator picks "New instance". Every other launch shows
    // the panel before entering the event loop.
    if prompt_window.borrow().is_none() {
        app.window().show()?;
    }

    let result = slint::run_event_loop();
    timer.stop();
    quit_timer.stop();
    // Remove the notification-area icon before dropping its component; a
    // hidden tray icon must not survive the process exit path.
    if let Some(tray_icon) = tray.borrow().as_ref() {
        tray_icon.set_tray_visible(false);
    }
    *tray.borrow_mut() = None;
    if let Some(prompt) = prompt_window.borrow_mut().take() {
        let _ = prompt.hide();
    }
    let _ = app.window().hide();
    // Stop the pre-warmed picker helper (including an open dialog) before the
    // process exits; otherwise PowerShell would outlive the panel.
    state.borrow().folder_picker.shutdown();
    if let Some(mut watcher) = theme_watcher_slot.borrow_mut().take() {
        watcher.stop();
    }
    controller.borrow_mut().shutdown();
    result?;
    Ok(())
}

/// How a panel launch begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchMode {
    /// Check for a running panel and ask the operator what to do.
    Normal,
    /// Ask the running panel to show its window, then exit. Never creates one.
    ActivateOnly,
    /// Start without the "already running" prompt (`--new-instance`).
    NewInstance,
}

/// Parse the launch flags. Only the exact long options are recognised; an
/// unknown argument is ignored exactly like before, so a stray shell word does
/// not change the startup decision.
fn launch_mode(arguments: &[String]) -> LaunchMode {
    let has = |flag: &str| arguments.iter().any(|argument| argument == flag);
    // `--activate-only` is the more specific request: it must not be turned
    // into a full launch by an accompanying `--new-instance`.
    if has("--activate-only") {
        LaunchMode::ActivateOnly
    } else if has("--new-instance") {
        LaunchMode::NewInstance
    } else {
        LaunchMode::Normal
    }
}

/// Check for another panel without registering this one.
///
/// The detection is best effort: without a usable state directory the panel
/// starts normally instead of refusing to run, and a stale lock of a crashed
/// panel is cleaned up by [`single_instance::Owner::detect`]. Returns an error
/// string (already localised when it comes from [`Text`]) that the caller logs.
fn check_launch(state: &mut UiState, slot: &Rc<RefCell<Option<Owner>>>) -> Result<(), String> {
    let Some(dir) = user_state::state_dir() else {
        return Err(state.text.instance_check_unavailable().to_string());
    };
    let detected = Owner::detect(&dir, single_instance::now_millis());
    let Some(background) = detected else {
        register_single_instance(state, slot);
        return Ok(());
    };
    // Being registered now would make the second launch look like an owner, so
    // the lock is written only when the operator picks "New instance".
    state.single_instance_prompt = Some(SingleInstancePrompt {
        background,
        request: None,
    });
    Ok(())
}

/// Register this process as the lock owner (idempotent).
fn register_single_instance(state: &mut UiState, slot: &Rc<RefCell<Option<Owner>>>) {
    let mut slot = slot.borrow_mut();
    if slot.is_some() {
        return;
    }
    let Some(dir) = user_state::state_dir() else {
        let message = state.text.instance_check_unavailable().to_string();
        state.push_log(LogLevel::Warn, message);
        return;
    };
    *slot = Some(Owner::register(&dir));
}

/// Per-tick single-instance work: heartbeat, answering other launches, and the
/// progress of this launch's own activation request.
///
/// Returns `true` when the event loop has been asked to quit, so the caller can
/// skip the rest of the tick. Runs before the server status work: raising a
/// hidden prompt window must not be delayed by transfer bookkeeping.
fn service_single_instance(
    app: &AppWindow,
    prompt_window: &Rc<RefCell<Option<InstancePromptWindow>>>,
    state: &mut UiState,
    slot: &Rc<RefCell<Option<Owner>>>,
    tray: &Rc<RefCell<Option<TrayIcon>>>,
) -> bool {
    let now = single_instance::now_millis();
    if let Some(owner) = slot.borrow_mut().as_mut() {
        // A failed refresh is reported to stderr by the protocol module; the
        // running server is unaffected either way, so nothing else to do here.
        owner.heartbeat(now);
        // Answer another launch only when this panel has a main window of its
        // own. While the "already running" prompt is open, this process has not
        // started yet and must not claim the request.
        let prompt_open = state.single_instance_prompt.is_some();
        if !prompt_open {
            if let Some(request) = owner.claim_activation(now) {
                let message = state.text.activated_from_another_instance(request.pid);
                state.push_log(LogLevel::Info, message);
                raise_window(app, state, tray);
            }
        }
    }

    let Some(prompt_state) = state.single_instance_prompt.as_mut() else {
        return false;
    };
    let request = prompt_state.request.take();
    let Some(request) = request else {
        return false;
    };
    let Some(prompt) = prompt_window
        .borrow()
        .as_ref()
        .map(InstancePromptWindow::clone_strong)
    else {
        prompt_state.request = Some(request);
        return false;
    };
    let Some(dir) = user_state::state_dir() else {
        prompt.set_waiting(false);
        prompt.set_status_text(state.text.instance_not_responding().into());
        return false;
    };
    if request.poll_ack(&dir) {
        prompt.set_waiting(false);
        // The running panel raised its own window; this launch is done.
        let _ = slint::quit_event_loop();
        return true;
    }
    if request.expired(now) {
        request.discard();
        prompt.set_waiting(false);
        prompt.set_status_text(state.text.instance_not_responding().into());
        return false;
    }
    // Keep the request alive until the answer arrives or the deadline passes.
    prompt_state.request = Some(request);
    false
}

/// Show and raise the panel after another launch asked for it.
///
/// Restores a minimised window, reveals a window that was hidden by "Run in
/// background" and puts the tray icon away. The operating system may still
/// decline to hand over the foreground (Windows only flashes the task-bar
/// button when the request comes from a background process); the window is then
/// visible and the task-bar entry is the fallback.
fn raise_window(app: &AppWindow, state: &mut UiState, tray: &Rc<RefCell<Option<TrayIcon>>>) {
    let tray_icon = tray.borrow().as_ref().map(TrayIcon::clone_strong);
    show_main_window(app, state, tray_icon.as_ref());
}

/// Reveal the main window and clear every state that belongs to the hidden
/// background mode.
fn show_main_window(app: &AppWindow, state: &mut UiState, tray_icon: Option<&TrayIcon>) {
    state.close_prompt_open = false;
    state.pending_background = false;
    app.set_close_prompt_open(false);
    let window = app.window();
    window.set_minimized(false);
    if let Err(error) = window.show() {
        let message = state.text.window_show_failed(&error.to_string());
        state.push_log(LogLevel::Warn, message);
    }
    // Hide the tray icon only after the window is visible: the event loop must
    // never see both surfaces hidden.
    if let Some(tray_icon) = tray_icon {
        tray_icon.set_tray_visible(false);
    }
}

/// Create the notification-area icon component, if this build has a native
/// tray backend.
///
/// The icon image and tooltip are assigned immediately: Slint creates the
/// platform handle on the first event-loop turn and does not retry after a
/// failed `CreateIconIndirect`/`NIM_ADD`.
fn create_tray(text: Text) -> Option<TrayIcon> {
    match TrayIcon::new() {
        Ok(tray) => {
            tray.set_tray_icon(app_icon());
            tray.set_tray_tooltip(text.tray_tooltip().into());
            Some(tray)
        }
        Err(error) => {
            eprintln!("warning: cannot create the notification-area icon: {error}");
            None
        }
    }
}

/// Create the tray component on first use and wire its callbacks.
fn ensure_tray(
    app: &AppWindow,
    state: &Rc<RefCell<UiState>>,
    tray: &Rc<RefCell<Option<TrayIcon>>>,
) -> bool {
    if tray.borrow().is_some() {
        return true;
    }
    let Some(tray_icon) = create_tray(state.borrow().text) else {
        let message = state.borrow().text.background_unavailable();
        state.borrow_mut().push_log(LogLevel::Warn, message);
        return false;
    };
    *tray.borrow_mut() = Some(tray_icon);
    install_tray_callbacks(app, state, tray);
    true
}

/// Wire the tray icon's click and menu entries to the panel.
///
/// The callbacks capture a weak handle of the tray itself: a strong reference
/// would make the component own its own callback and leak the icon.
fn install_tray_callbacks(
    app: &AppWindow,
    state: &Rc<RefCell<UiState>>,
    tray: &Rc<RefCell<Option<TrayIcon>>>,
) {
    let Some(tray_icon) = tray.borrow().as_ref().map(TrayIcon::clone_strong) else {
        return;
    };
    let weak_app = app.as_weak();
    let state_show = state.clone();
    let weak_tray = tray_icon.as_weak();
    tray_icon.on_show_requested(move || {
        let (Some(app), Some(tray), Ok(mut state)) = (
            weak_app.upgrade(),
            weak_tray.upgrade(),
            state_show.try_borrow_mut(),
        ) else {
            return;
        };
        show_main_window(&app, &mut state, Some(&tray));
    });

    tray_icon.on_quit_requested(|| {
        let _ = slint::quit_event_loop();
    });
}

/// Ask the panel to move to the notification area.
///
/// The window is not hidden here: the tray's visibility change is queued and
/// the UI timer hides the window on its next tick. Returns `false` when there
/// is no usable tray component; the caller then keeps the window (and the close
/// prompt) in place instead of hiding the only UI.
fn enter_background(
    app: &AppWindow,
    state: &mut UiState,
    tray: &Rc<RefCell<Option<TrayIcon>>>,
) -> bool {
    let Some(tray_icon) = tray.borrow().as_ref().map(TrayIcon::clone_strong) else {
        let message = state.text.background_unavailable();
        state.push_log(LogLevel::Warn, message);
        return false;
    };
    state.close_prompt_open = false;
    app.set_close_prompt_open(false);
    // Refresh the icon and tooltip, then show the icon before the window is
    // hidden (the actual hide happens on the next UI tick).
    tray_icon.set_tray_icon(app_icon());
    tray_icon.set_tray_tooltip(state.text.tray_tooltip().into());
    tray_icon.set_tray_visible(true);
    state.pending_background = true;
    true
}

/// Handle a request to close the panel window.
///
/// With a running server this opens the three-way close prompt and keeps the
/// window visible; otherwise it exits and lets the normal shutdown path stop
/// whatever server state remains. Both the custom title-bar button and the
/// operating system's close request (Alt+F4) call this.
fn request_close(
    app: &AppWindow,
    controller: &Rc<RefCell<EmbeddedController>>,
    state: &Rc<RefCell<UiState>>,
) {
    let status = controller.borrow_mut().poll_status();
    let mut state = state.borrow_mut();
    if state.close_prompt_open {
        return;
    }
    if !server_is_active(&status) {
        let _ = slint::quit_event_loop();
        return;
    }
    let detail = app.get_bind_text().to_string();
    let detail = state.text.close_prompt_server_running(&detail);
    state.close_prompt_open = true;
    app.set_close_prompt_detail(detail.into());
    app.set_background_available(true);
    app.set_close_prompt_open(true);
}

/// Whether the embedded server is starting or running, i.e. closing the window
/// would stop serving clients.
fn server_is_active(status: &ServerStatus) -> bool {
    matches!(
        status,
        ServerStatus::Starting | ServerStatus::Running { .. }
    )
}

/// `--activate-only`: ask a running panel to show its window and exit.
///
/// Used by scripts and shortcuts that only want to bring the panel forward. The
/// request is written and polled here, before any window is created, so the
/// process never flashes a GUI.
fn activate_running_panel() -> Result<(), Box<dyn std::error::Error>> {
    let Some(dir) = user_state::state_dir() else {
        eprintln!("tftpd-r: no usable state directory for the single-instance check");
        return Ok(());
    };
    let Some(background) = Owner::detect(&dir, single_instance::now_millis()) else {
        eprintln!("tftpd-r: no running panel to activate");
        return Ok(());
    };
    let request = ActivationRequest::send(&dir, &background, single_instance::now_millis());
    let deadline = Instant::now() + single_instance::ACTIVATION_TIMEOUT;
    while Instant::now() < deadline {
        if request.poll_ack(&dir) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    eprintln!("tftpd-r: the running panel did not respond");
    Ok(())
}

fn spawn_discovery(slot: Arc<StdMutex<Option<Vec<IpAddr>>>>) {
    std::thread::spawn(move || {
        let addresses = netif::local_addresses();
        if let Ok(mut guard) = slot.lock() {
            *guard = Some(addresses);
        }
    });
}

/// Result of reading `config.toml` from the working directory.
enum InitialConfig {
    /// A valid configuration was loaded from disk.
    Loaded(ServerConfig),
    /// No `config.toml` exists; the panel starts from its own defaults.
    Missing,
    /// `config.toml` exists but is unreadable or invalid.
    Invalid(String),
}

impl InitialConfig {
    /// Configuration used for the settings form and the first start.
    ///
    /// Without a usable `config.toml`, the panel preselects "全部网络
    /// (0.0.0.0)": the desktop panel is normally used to share files on the
    /// LAN, and the server still only starts after the operator confirms the
    /// dialog and presses 启动. `ServerConfig::default()` and the daemon keep
    /// the conservative loopback default.
    ///
    /// A root loaded from `config.toml` always wins. Without one, the directory
    /// of the previous start (`remembered`) is restored when it still exists;
    /// otherwise the root stays unset and the panel shows "未指定根目录"
    /// instead of pretending that a `tftp-root` directory exists.
    fn config(&self, remembered: Option<&Path>) -> ServerConfig {
        match self {
            Self::Loaded(config) => config.clone(),
            Self::Missing | Self::Invalid(_) => {
                let mut config = gui_default_config();
                if let Some(root) = remembered.filter(|path| path.is_dir()) {
                    config.root_dir = root.to_path_buf();
                }
                config
            }
        }
    }

    /// Whether a valid configuration file was loaded.
    fn is_configured(&self) -> bool {
        matches!(self, Self::Loaded(_))
    }

    /// Error of an unreadable or invalid configuration file, for the log.
    fn error(&self) -> Option<&str> {
        match self {
            Self::Invalid(error) => Some(error),
            Self::Loaded(_) | Self::Missing => None,
        }
    }
}

/// Defaults of the panel's first-run form: safe server defaults plus the
/// wildcard bind address and **no root directory**. Core and daemon defaults
/// are not touched (`ServerConfig::default()` keeps its `tftp-root` path).
fn gui_default_config() -> ServerConfig {
    ServerConfig {
        bind_addresses: vec![IpAddr::V4(Ipv4Addr::UNSPECIFIED)],
        // The empty path is the GUI-only marker for "no root selected yet". It
        // is never passed to the server: `read_config` rejects it before a
        // start and asks the operator to choose a directory.
        root_dir: PathBuf::new(),
        ..ServerConfig::default()
    }
}

/// Load `config.toml` from the working directory.
fn load_initial_config() -> InitialConfig {
    match std::fs::read_to_string("config.toml") {
        Ok(text) => match ServerConfig::from_toml_str(&text) {
            Ok(config) => InitialConfig::Loaded(config),
            Err(error) => InitialConfig::Invalid(error.to_string()),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => InitialConfig::Missing,
        Err(error) => InitialConfig::Invalid(error.to_string()),
    }
}

/// Apply one click in the multi-select address picker.
///
/// "全部网络 (0.0.0.0)" is exclusive and sticky:
/// - selecting it clears every specific address;
/// - selecting a specific address clears it;
/// - unchecking it is not possible: the click keeps it selected, and selecting
///   it while it is already selected also drops any other selected address;
/// - unchecking the last specific address falls back to it, so the bind list can
///   never become empty.
fn toggle_address_choice(options: &mut [AddressChoice], address: IpAddr) {
    let wildcard = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
    let Some(index) = options.iter().position(|option| option.ip == address) else {
        return;
    };
    if address == wildcard {
        for option in options.iter_mut() {
            option.selected = option.ip == wildcard;
        }
        return;
    }
    let was_selected = options[index].selected;
    options[index].selected = !was_selected;
    if was_selected {
        if !options.iter().any(|option| option.selected) {
            if let Some(option) = options.iter_mut().find(|option| option.ip == wildcard) {
                option.selected = true;
            }
        }
    } else {
        if let Some(option) = options.iter_mut().find(|option| option.ip == wildcard) {
            option.selected = false;
        }
    }
}

fn poll_folder_picker(state: &mut UiState) -> Option<FolderPickOutcome> {
    let pending = state.pending_pick.as_ref()?;
    let picked = match pending.receiver.try_recv() {
        Ok(result) => result,
        Err(TryRecvError::Empty) => return None,
        // A disconnected worker means the dialog cannot report a selection;
        // treat it as a cancel so the UI does not stay in the pending state.
        Err(TryRecvError::Disconnected) => None,
    };
    let target = pending.target;
    let previous_hint = pending.previous_hint.clone();
    state.pending_pick = None;
    Some(FolderPickOutcome {
        target,
        picked,
        previous_hint,
    })
}

fn poll_log_export(state: &mut UiState) -> Option<(SaveChoice, String)> {
    let choice = match state.log_export.as_ref()?.receiver.try_recv() {
        Ok(choice) => choice,
        Err(TryRecvError::Empty) => return None,
        Err(TryRecvError::Disconnected) => SaveChoice::Unavailable,
    };
    let text = state.log_export.take()?.text;
    Some((choice, text))
}

fn handle_folder_pick(
    app: &AppWindow,
    state: &mut UiState,
    picked: Option<PathBuf>,
    previous_hint: &str,
) {
    match picked {
        Some(path) if path.is_dir() => {
            let display = display_root_path(&path);
            // Only the form is updated here: the directory becomes the
            // configured root when the dialog is saved, so Cancel still
            // restores the value that is currently in effect.
            let _ = state.recent.record(&path);
            app.set_settings_root(display.clone().into());
            // The log is behind the modal dialog, so repeat the "not saved yet"
            // hint in the form itself. Picking a directory only edits the form;
            // "Save settings" (or the recent-root menu's "Select…") applies it.
            app.set_settings_hint(state.text.root_selected_not_saved().into());
            let message = state.text.root_selected_in_form(&display);
            state.push_log(LogLevel::Info, message);
        }
        Some(path) => {
            let message = state.text.path_is_not_a_directory(&display_path(&path));
            state.push_log(LogLevel::Error, message);
            // Remove the "opening…" hint without discarding an older reminder.
            app.set_settings_hint(previous_hint.into());
        }
        // Cancelled dialog or unavailable helper: restore the form as it was.
        None => app.set_settings_hint(previous_hint.into()),
    }
    refresh_models(app, state);
}

/// Apply a root directory picked from the recent-root menu.
///
/// This is the same operation as choosing an entry in the list: the running
/// server is restarted when no transfer is active, otherwise the new root is
/// only stored until the operator restarts manually.
fn handle_root_pick(
    app: &AppWindow,
    controller: &Rc<RefCell<EmbeddedController>>,
    state: &mut UiState,
    picked: Option<PathBuf>,
) {
    match picked {
        Some(path) if path.is_dir() => switch_root(app, controller, state, path),
        Some(path) => {
            let message = state.text.root_unavailable(&display_path(&path));
            state.push_log(LogLevel::Error, message);
            refresh_models(app, state);
        }
        None => {}
    }
}

/// Store `path` as the configured root, in the recent list and in the
/// remembered-root file. Returns the display form used by the UI and log.
fn store_root(state: &mut UiState, path: PathBuf) -> String {
    state.config.root_dir = path.clone();
    let _ = state.recent.record(&path);
    state.remember_root();
    display_root_path(&path)
}

/// Store `path` as the configured root, remember it and apply it to a running
/// idle server. Shared by the recent-root list and its "Select…" button.
fn switch_root(
    app: &AppWindow,
    controller: &Rc<RefCell<EmbeddedController>>,
    state: &mut UiState,
    path: PathBuf,
) {
    let root_text = store_root(state, path);
    app.set_settings_root(root_text.clone().into());
    let config = state.config.clone();
    match auto_restart_if_idle(controller, state, &config) {
        RestartDisposition::Restarted => {
            let message = state.text.root_switched_restarting(&root_text);
            state.push_log(LogLevel::Info, message);
        }
        RestartDisposition::Deferred { transfers } => {
            let message = state.text.root_switched_deferred(&root_text, transfers);
            state.push_log(LogLevel::Warn, message);
        }
        RestartDisposition::Failed => {
            let message = state.text.root_switched_restart_failed(&root_text);
            state.push_log(LogLevel::Error, message);
        }
        RestartDisposition::Unchanged => {
            let message = state.text.root_unchanged(&root_text);
            state.push_log(LogLevel::Info, message);
        }
        RestartDisposition::NotRunning => {
            let message = state.text.root_stored_for_next_start(&root_text);
            state.push_log(LogLevel::Info, message);
        }
    }
    refresh_models(app, state);
    let status = controller.borrow_mut().poll_status();
    apply_status(app, &status, state);
}

/// Write an explicitly requested log export and report the result.
fn finish_log_export(app: &AppWindow, state: &mut UiState, choice: SaveChoice, text: String) {
    match choice {
        SaveChoice::Path(path) => match write_export(&path, text.as_str()) {
            Ok(()) => {
                let message = state.text.log_exported(&display_path(&path));
                state.push_log(LogLevel::Info, message);
            }
            Err(error) => {
                let message = state.text.log_export_failed(&error.to_string());
                state.push_log(LogLevel::Error, message);
            }
        },
        SaveChoice::Cancelled => {}
        SaveChoice::Unavailable => {
            let message = state.text.log_export_unavailable();
            state.push_log(LogLevel::Error, message);
        }
    }
    refresh_models(app, state);
}

fn install_callbacks(
    app: &AppWindow,
    controller: &Rc<RefCell<EmbeddedController>>,
    state: &Rc<RefCell<UiState>>,
    theme_watcher: &Rc<RefCell<Option<SystemThemeWatcher>>>,
    tray: &Rc<RefCell<Option<TrayIcon>>>,
) {
    // Start / restart: the same button changes meaning while the server runs.
    {
        let weak = app.as_weak();
        let controller = controller.clone();
        let state = state.clone();
        app.on_primary_requested(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            let config = match read_config(&app, &state.config, state.text) {
                Ok(config) => config,
                Err(message) => {
                    app.set_settings_error(message.clone().into());
                    let log = state.text.settings_invalid(&message);
                    state.push_log(LogLevel::Error, log);
                    // The problematic value lives in the dialog; open it so the
                    // operator can fix it (for example pick a root directory).
                    app.set_settings_open(true);
                    refresh_models(&app, &mut state);
                    return;
                }
            };
            let status = controller.borrow_mut().poll_status();
            let running = controller.borrow().is_running();
            let needs_restart = !matches!(status, ServerStatus::Stopped);
            let result = if needs_restart {
                controller.borrow_mut().restart(config.clone())
            } else {
                controller.borrow_mut().start(config.clone())
            };
            match result {
                Ok(()) => {
                    state.config = config.clone();
                    state.failed_message = None;
                    state.running_root = Some(state.config.root_dir.clone());
                    state.running_config = Some(config);
                    state.remember_root();
                    if running {
                        // The restart will pass through Stopping before the new
                        // server is ready; force the transition log for that.
                        state.last_status_key = status_key(&ServerStatus::Stopping);
                    }
                    let root = state.config.root_dir.clone();
                    let _ = state.recent.record(&root);
                    let bind = state.config.bind_summary();
                    let root = display_root_path(&state.config.root_dir);
                    let message = state.text.service_starting(running, &bind, &root);
                    state.push_log(LogLevel::Info, message);
                    app.set_settings_error("".into());
                    app.set_settings_hint("".into());
                }
                Err(error) => {
                    let message = error.to_string();
                    state.failed_message = Some(message.clone());
                    let log = state.text.service_start_failed(running, &message);
                    state.push_log(LogLevel::Error, log);
                }
            }
            refresh_models(&app, &mut state);
            let status = controller.borrow_mut().poll_status();
            apply_status(&app, &status, &state);
        });
    }
    // Stop.
    {
        let weak = app.as_weak();
        let controller = controller.clone();
        let state = state.clone();
        app.on_stop_requested(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            if controller.borrow_mut().stop() {
                let message = state.text.service_stopping();
                state.push_log(LogLevel::Info, message);
            }
            refresh_models(&app, &mut state);
            let status = controller.borrow_mut().poll_status();
            apply_status(&app, &status, &state);
        });
    }
    // Apply settings.
    {
        let weak = app.as_weak();
        let controller = controller.clone();
        let state = state.clone();
        app.on_apply_settings(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            match read_config(&app, &state.config, state.text) {
                Ok(config) => {
                    state.config = config.clone();
                    state.remember_root();
                    let root = state.config.root_dir.clone();
                    let _ = state.recent.record(&root);
                    match auto_restart_if_idle(&controller, &mut state, &config) {
                        RestartDisposition::Restarted => {
                            let message = state.text.settings_saved_restarting();
                            state.push_log(LogLevel::Info, message);
                        }
                        RestartDisposition::Deferred { transfers } => {
                            let message = state.text.settings_saved_deferred(transfers);
                            state.push_log(LogLevel::Warn, message);
                        }
                        RestartDisposition::Failed => {
                            let message = state.text.settings_saved_restart_failed();
                            state.push_log(LogLevel::Error, message);
                        }
                        RestartDisposition::NotRunning => {
                            let message = state.text.settings_saved_next_start();
                            state.push_log(LogLevel::Info, message);
                        }
                        RestartDisposition::Unchanged => {
                            let message = state.text.settings_unchanged();
                            state.push_log(LogLevel::Info, message);
                        }
                    }
                    app.set_settings_error("".into());
                    app.set_settings_hint("".into());
                    app.set_settings_open(false);
                }
                Err(message) => {
                    app.set_settings_error(message.clone().into());
                    let log = state.text.settings_invalid(&message);
                    state.push_log(LogLevel::Error, log);
                }
            }
            refresh_models(&app, &mut state);
            let status = controller.borrow_mut().poll_status();
            apply_status(&app, &status, &state);
        });
    }
    // Cancel settings: restore the values that are currently in effect.
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_cancel_settings(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            apply_config_to_settings(&app, &mut state);
            app.set_settings_error("".into());
            app.set_settings_hint("".into());
            app.set_settings_open(false);
        });
    }
    // Toggle one bind address in the settings picker. "全部网络" is exclusive
    // and never unchecks, and the bind list can never become empty; see
    // `toggle_address_choice`. The dialog stays open so several addresses can
    // be selected in one go.
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_address_toggled(move |address| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let Ok(address) = address.to_string().parse::<IpAddr>() else {
                return;
            };
            let mut state = state.borrow_mut();
            toggle_address_choice(&mut state.address_options, address);
            apply_address_options(&app, &state);
        });
    }
    // Native folder picker for the settings root field. The dialog runs in a
    // pre-warmed helper process on a background thread; the result arrives
    // through the timer. Opening a picker is internal UI activity and is not
    // logged.
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_browse_root(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            if state.pending_pick.is_some() {
                return;
            }
            let typed = PathBuf::from(app.get_settings_root().trim());
            let initial = if typed.is_dir() {
                Some(typed)
            } else if root_is_set(&state.config.root_dir) && state.config.root_dir.is_dir() {
                Some(state.config.root_dir.clone())
            } else {
                None
            };
            let previous_hint = app.get_settings_hint().to_string();
            let receiver = state.folder_picker.request(initial);
            state.pending_pick = Some(PendingFolderPick {
                target: FolderPickTarget::SettingsForm,
                previous_hint,
                receiver,
            });
            // The dialog is not visible yet while the pre-warmed helper is
            // still loading; show that the click was registered instead of
            // leaving the form unchanged for a moment.
            app.set_settings_hint(state.text.opening_folder_picker().into());
        });
    }
    // Native folder picker opened from the recent-root menu. Unlike the form
    // field above, the chosen directory is applied immediately (and restarts an
    // idle running server) without passing through "Save settings".
    {
        let state = state.clone();
        app.on_browse_root_menu(move || {
            let mut state = state.borrow_mut();
            if state.pending_pick.is_some() {
                return;
            }
            let initial = if root_is_set(&state.config.root_dir) && state.config.root_dir.is_dir() {
                Some(state.config.root_dir.clone())
            } else {
                None
            };
            let receiver = state.folder_picker.request(initial);
            state.pending_pick = Some(PendingFolderPick {
                target: FolderPickTarget::Root,
                previous_hint: String::new(),
                receiver,
            });
        });
    }
    // Filter the log table. The retained entries (and the export) are not
    // changed; the choice only controls what is displayed.
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_log_filter_changed(move |filter| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            let next = LogFilter::from_key(filter.as_str());
            if next != state.log_filter {
                state.log_filter = next;
                state.logs_dirty = true;
                refresh_models(&app, &mut state);
            }
            app.set_log_filter(state.log_filter.key().into());
        });
    }
    // Export the retained log through a native save dialog. The snapshot is
    // taken when the button is pressed; the result is polled by the UI timer.
    {
        let state = state.clone();
        app.on_export_logs(move || {
            let mut state = state.borrow_mut();
            if state.log_export.is_some() {
                return;
            }
            let now = Local::now().fixed_offset();
            let text = format_export(&state.logs, env!("CARGO_PKG_VERSION"), now, state.text);
            state.log_export = Some(PendingLogExport {
                receiver: log_export::save_log_async(export_file_name(now), state.text),
                text,
            });
        });
    }
    // Clear the log view. Clearing is silent (a "cleared" row would reappear in
    // the just-emptied panel); the filter choice is kept.
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_clear_logs(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            state.logs.clear();
            state.logs_dirty = true;
            refresh_models(&app, &mut state);
        });
    }
    // Pick a different root directory from the recent list.
    {
        let weak = app.as_weak();
        let controller = controller.clone();
        let state = state.clone();
        app.on_recent_selected(move |path_text| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            let path = PathBuf::from(path_text.as_str());
            if !path.is_dir() {
                let message = state.text.root_unavailable(&display_path(&path));
                state.push_log(LogLevel::Error, message);
                refresh_models(&app, &mut state);
                return;
            }
            switch_root(&app, &controller, &mut state, path);
        });
    }

    // Fill the settings root field from its history drop-down. Unlike the
    // recent-root menu this only edits the form: the directory becomes active
    // when Save settings is pressed, so Cancel still restores the active root.
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_settings_recent_selected(move |path_text| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            let path = PathBuf::from(path_text.as_str());
            if !path.is_dir() {
                let message = state.text.path_is_not_a_directory(&display_path(&path));
                state.push_log(LogLevel::Error, message.clone());
                // The history is behind the modal dialog; repeat the failure in
                // the form so a directory that disappeared while the list was
                // open is not silently ignored.
                app.set_settings_error(message.into());
                refresh_models(&app, &mut state);
                return;
            }
            let display = display_root_path(&path);
            app.set_settings_root(display.clone().into());
            app.set_settings_hint(state.text.root_selected_not_saved().into());
            let message = state.text.root_selected_in_form(&display);
            state.push_log(LogLevel::Info, message);
            refresh_models(&app, &mut state);
        });
    }
    // Forget the recent-root history. The configured and running root are
    // unaffected: they are server configuration, not history.
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_clear_recent(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            state.recent.clear();
            let message = state.text.recent_cleared();
            state.push_log(LogLevel::Info, message);
            refresh_models(&app, &mut state);
        });
    }

    // Appearance: follow the system, light or dark. This is a local display
    // preference, so it applies immediately and never restarts the server.
    {
        let weak = app.as_weak();
        let state = state.clone();
        let theme_watcher = theme_watcher.clone();
        app.on_theme_mode_changed(move |value| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let Some(mode) = ThemeMode::from_slint_value(value) else {
                return;
            };
            let mut state = state.borrow_mut();
            if state.theme_mode == mode {
                app.set_theme_mode(mode.slint_value());
                return;
            }
            state.theme_mode = mode;
            if let Err(error) = state.prefs.set_theme_mode(mode) {
                let message = state.text.theme_preference_not_saved(&error.to_string());
                state.push_log(LogLevel::Warn, message);
            }
            // Probing the operating system costs a helper process; only do it
            // while the panel actually follows the system.
            if let Some(watcher) = theme_watcher.borrow().as_ref() {
                watcher.set_enabled(mode == ThemeMode::System);
            }
            apply_theme(&app, &state);
            app.set_theme_mode(mode.slint_value());
            refresh_models(&app, &mut state);
        });
    }
    // Language: persist the choice for the next start. The running panel keeps
    // its language (the activity log stores rendered lines and the native dialog
    // titles were captured at startup), so no property is re-translated here.
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_language_mode_changed(move |value| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let Some(preference) = LanguagePreference::from_slint_value(value) else {
                return;
            };
            let mut state = state.borrow_mut();
            if state.prefs.language_preference() == preference {
                app.set_language_mode(preference.slint_value());
                return;
            }
            let name = state.text.language_preference_name(preference).to_string();
            if let Err(error) = state.prefs.set_language_preference(preference) {
                let message = state.text.language_preference_not_saved(&error.to_string());
                state.push_log(LogLevel::Warn, message);
            } else {
                let message = state.text.language_preference_saved(&name);
                state.push_log(LogLevel::Info, message);
            }
            app.set_language_mode(preference.slint_value());
            refresh_models(&app, &mut state);
        });
    }
    // About dialog: open the fixed project URL in the default browser. The URL
    // comes from Cargo metadata, so no user-controlled input reaches the
    // platform handler; a failed launch is reported in the activity log.
    {
        let state = state.clone();
        app.on_open_repository_requested(move || {
            if let Err(error) = open_repository_url() {
                let text = state.borrow().text;
                let message = text.repository_open_failed(&error.to_string());
                state.borrow_mut().push_log(LogLevel::Error, message);
            }
        });
    }
    // Window controls of the frameless title bar. Use the window API rather
    // than the Slint window properties so the OS state stays authoritative.
    {
        let weak = app.as_weak();
        app.on_minimize_requested(move || {
            if let Some(app) = weak.upgrade() {
                app.window().set_minimized(true);
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_maximize_requested(move || {
            if let Some(app) = weak.upgrade() {
                let window = app.window();
                window.set_maximized(!window.is_maximized());
            }
        });
    }
    // Close button: ask before stopping a running server. The operating
    // system's close request (Alt+F4) goes through the same helper so it cannot
    // bypass the prompt.
    {
        let weak = app.as_weak();
        let controller = controller.clone();
        let state = state.clone();
        app.on_close_requested(move || {
            if let Some(app) = weak.upgrade() {
                request_close(&app, &controller, &state);
            }
        });
    }
    {
        let weak = app.as_weak();
        let controller = controller.clone();
        let state = state.clone();
        app.window().on_close_requested(move || {
            let Some(app) = weak.upgrade() else {
                return CloseRequestResponse::HideWindow;
            };
            request_close(&app, &controller, &state);
            CloseRequestResponse::KeepWindowShown
        });
    }

    // "Close TftpdR?" choices. "Close and exit" hides the window before the
    // event loop stops; the normal shutdown path then drains the server.
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_close_and_exit_requested(move || {
            if let Some(app) = weak.upgrade() {
                let mut state = state.borrow_mut();
                state.close_prompt_open = false;
                state.pending_background = false;
                app.set_close_prompt_open(false);
                let _ = app.window().hide();
            }
            let _ = slint::quit_event_loop();
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        let tray = tray.clone();
        app.on_background_requested(move || {
            if let Some(app) = weak.upgrade() {
                if ensure_tray(&app, &state, &tray) {
                    let mut state = state.borrow_mut();
                    enter_background(&app, &mut state, &tray);
                }
            }
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_close_prompt_cancel_requested(move || {
            if let Some(app) = weak.upgrade() {
                let mut state = state.borrow_mut();
                state.close_prompt_open = false;
                state.pending_background = false;
                app.set_close_prompt_open(false);
            }
        });
    }
}

/// Wire the second-launch prompt window.
///
/// The prompt never starts a server unless the operator explicitly picks "New
/// instance"; asking the running panel for its window exits after the
/// acknowledgement. "New instance" shows the main panel before hiding the
/// prompt, so the event loop always has a visible window during the switch.
fn install_instance_prompt_callbacks(
    prompt: &InstancePromptWindow,
    app: &AppWindow,
    state: &Rc<RefCell<UiState>>,
    single_instance: &Rc<RefCell<Option<Owner>>>,
    prompt_window: &Rc<RefCell<Option<InstancePromptWindow>>>,
) {
    {
        let weak_prompt = prompt.as_weak();
        let state = state.clone();
        prompt.on_activate_requested(move || {
            let Some(prompt) = weak_prompt.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            let text = state.text;
            let Some(instance) = state.single_instance_prompt.as_mut() else {
                return;
            };
            if instance.request.is_some() {
                return;
            }
            let Some(dir) = user_state::state_dir() else {
                prompt.set_status_text(text.instance_not_responding().into());
                return;
            };
            let background = instance.background.clone();
            instance.request = Some(ActivationRequest::send(
                &dir,
                &background,
                single_instance::now_millis(),
            ));
            prompt.set_waiting(true);
            prompt.set_status_text(text.switching_to_existing().into());
        });
    }
    {
        let weak_prompt = prompt.as_weak();
        let weak_app = app.as_weak();
        let state = state.clone();
        let single_instance = single_instance.clone();
        let prompt_window = prompt_window.clone();
        prompt.on_new_instance_requested(move || {
            let Some(prompt) = weak_prompt.upgrade() else {
                return;
            };
            let Some(app) = weak_app.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            state.single_instance_prompt = None;
            register_single_instance(&mut state, &single_instance);
            // Show the panel first: the event loop must never see both windows
            // hidden, or it would stop before the new instance is usable.
            app.window().show().ok();
            prompt.hide().ok();
            *prompt_window.borrow_mut() = None;
            let message = state.text.panel_started(env!("CARGO_PKG_VERSION"));
            state.push_log(LogLevel::Info, message);
        });
    }
    {
        prompt.on_cancel_requested(|| {
            let _ = slint::quit_event_loop();
        });
    }
    {
        prompt.window().on_close_requested(|| {
            let _ = slint::quit_event_loop();
            CloseRequestResponse::KeepWindowShown
        });
    }
}

fn read_config(
    app: &AppWindow,
    fallback: &ServerConfig,
    text: Text,
) -> Result<ServerConfig, String> {
    let port_text = app.get_settings_port().trim().to_string();
    let port: u16 = port_text
        .parse()
        .map_err(|_| text.port_invalid(&port_text))?;
    if port == 0 {
        return Err(text.port_out_of_range().to_string());
    }
    let mut bind_addresses = Vec::new();
    for option in app.get_settings_address_options().iter() {
        if !option.selected {
            continue;
        }
        let address: IpAddr = option
            .address
            .parse()
            .map_err(|_| text.bind_address_invalid(&option.address))?;
        if !bind_addresses.contains(&address) {
            bind_addresses.push(address);
        }
    }
    if bind_addresses.is_empty() {
        return Err(text.select_bind_address().to_string());
    }
    let root_dir = root_from_text(&app.get_settings_root(), text)?;
    let block_size: u16 = parse_setting(
        &app.get_settings_block_size(),
        text.label_block_size(),
        text,
    )?;
    let timeout_secs: u64 = parse_setting(&app.get_settings_timeout(), text.label_timeout(), text)?;
    let max_sessions: usize = parse_setting(
        &app.get_settings_max_sessions(),
        text.label_max_sessions(),
        text,
    )?;
    let config = ServerConfig {
        port,
        bind_addresses,
        root_dir,
        block_size,
        timeout_secs,
        max_sessions,
        ..fallback.clone()
    };
    config
        .validate()
        .map_err(|error| text.config_validation_failed(&error.to_string()))?;
    Ok(config)
}

/// Root directory from the settings field.
///
/// An empty field means the operator has not selected a root directory at all;
/// starting the server must ask for one instead of trying to resolve an empty
/// path (or a dummy placeholder directory).
fn root_from_text(text: &str, messages: Text) -> Result<PathBuf, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(messages.choose_root_first().to_string());
    }
    Ok(PathBuf::from(text))
}

fn parse_setting<T: std::str::FromStr>(
    text: &str,
    label: &str,
    messages: Text,
) -> Result<T, String> {
    let text = text.trim();
    text.parse::<T>()
        .map_err(|_| messages.number_invalid(label, text))
}

fn apply_config_to_settings(app: &AppWindow, state: &mut UiState) {
    let config = state.config.clone();
    app.set_settings_port(config.port.to_string().into());
    // No root selected means an empty field, never the core default path.
    app.set_settings_root(display_root_or_empty(&config.root_dir).into());
    app.set_settings_block_size(config.block_size.to_string().into());
    app.set_settings_timeout(config.timeout_secs.to_string().into());
    app.set_settings_max_sessions(config.max_sessions.to_string().into());
    let discovered = state.discovered_snapshot().unwrap_or_default();
    state.rebuild_address_options(&config.bind_addresses, &discovered);
    apply_address_options(app, state);
}

fn apply_address_options(app: &AppWindow, state: &UiState) {
    let rows: Vec<AddressOption> = state
        .address_options
        .iter()
        .map(|option| AddressOption {
            label: option.label.clone().into(),
            address: option.ip.to_string().into(),
            selected: option.selected,
        })
        .collect();
    app.set_settings_address_options(ModelRc::new(VecModel::from(rows)));
    app.set_settings_bind_summary(state.bind_summary().into());
}

fn address_label(ip: IpAddr, text: Text) -> String {
    match ip {
        IpAddr::V4(ip) if ip.is_unspecified() => text.all_networks().to_string(),
        IpAddr::V4(ip) if ip == Ipv4Addr::LOCALHOST => text.loopback().to_string(),
        IpAddr::V6(ip) if ip.is_unspecified() => text.all_networks_ipv6().to_string(),
        IpAddr::V6(ip) if ip == Ipv6Addr::LOCALHOST => text.loopback_ipv6().to_string(),
        other => other.to_string(),
    }
}

/// Push the resolved colour scheme into the Slint window.
///
/// The window copies `dark-mode` into the `Theme` global and the style
/// `Palette`, so the custom tokens and the ScrollView scrollbars follow the
/// same choice.
fn apply_theme(app: &AppWindow, state: &UiState) {
    app.set_dark_mode(state.is_dark());
    app.set_theme_mode(state.theme_mode.slint_value());
}

fn apply_status(app: &AppWindow, status: &ServerStatus, state: &UiState) {
    let text = state.text;
    let (status_text, kind, detail, primary_enabled, primary_is_restart, stop_enabled) =
        match status {
            ServerStatus::Starting => (
                text.status_starting().to_string(),
                2,
                String::new(),
                false,
                false,
                true,
            ),
            ServerStatus::Running { .. } => (
                text.status_running().to_string(),
                1,
                String::new(),
                true,
                true,
                true,
            ),
            ServerStatus::Stopping => (
                text.status_stopping().to_string(),
                3,
                String::new(),
                false,
                false,
                false,
            ),
            ServerStatus::Stopped => match &state.failed_message {
                Some(message) => (
                    text.status_start_failed().to_string(),
                    4,
                    message.clone(),
                    true,
                    false,
                    false,
                ),
                None => (
                    text.status_stopped().to_string(),
                    0,
                    String::new(),
                    true,
                    false,
                    false,
                ),
            },
            ServerStatus::Failed { message } => (
                text.status_error().to_string(),
                4,
                message.clone(),
                true,
                false,
                false,
            ),
        };
    let bind = match status {
        ServerStatus::Running { local_addrs } => summarize_addrs(local_addrs, text),
        _ => state.config.bind_summary(),
    };
    // The status bar only shows the last directory name; the full path stays in
    // the settings dialog, the log and the recent-directory menu. A panel
    // without a root directory says so instead of showing a placeholder path.
    let root = match status {
        ServerStatus::Starting | ServerStatus::Running { .. } | ServerStatus::Stopping => {
            let path = state
                .running_root
                .as_deref()
                .unwrap_or(&state.config.root_dir);
            root_status_text(path, text)
        }
        _ => root_status_text(&state.config.root_dir, text),
    };
    app.set_status_text(status_text.into());
    app.set_status_kind(kind);
    app.set_status_detail(detail.into());
    app.set_primary_enabled(primary_enabled);
    app.set_primary_is_restart(primary_is_restart);
    app.set_stop_enabled(stop_enabled);
    app.set_bind_text(bind.into());
    app.set_root_text(root.into());
}

/// Whether a root directory has been selected.
///
/// The GUI uses an empty path as its "not selected" marker; the server core
/// never sees it because starting requires a selected directory.
fn root_is_set(path: &Path) -> bool {
    !path.as_os_str().is_empty()
}

/// Text for the settings root field: empty when no directory is selected.
fn display_root_or_empty(path: &Path) -> String {
    if root_is_set(path) {
        display_root_path(path)
    } else {
        String::new()
    }
}

/// Text for the compact status bar: the leaf directory name, or an explicit
/// "no root directory" instead of a dummy path when nothing is selected.
fn root_status_text(path: &Path, text: Text) -> String {
    if root_is_set(path) {
        root_display_name(path)
    } else {
        text.root_unset().to_string()
    }
}

/// Display a root directory as the absolute, canonical path when it exists.
/// The configured value may be relative to the panel's working directory.
fn display_root_path(path: &Path) -> String {
    std::fs::canonicalize(path).map_or_else(|_| display_path(path), |path| display_path(&path))
}

/// Last directory name for the compact status bar. A filesystem root has no
/// file name and keeps its full display path.
fn root_display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || display_root_path(path),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Compact comma-separated address list for the status bar and log.
fn summarize_addrs(addrs: &[SocketAddr], text: Text) -> String {
    match addrs.len() {
        0 => text.dash().to_string(),
        1..=3 => addrs
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(text.list_separator()),
        count => text.listener_count_summary(&addrs[0].to_string(), count),
    }
}

fn status_key(status: &ServerStatus) -> i32 {
    match status {
        ServerStatus::Starting => 2,
        ServerStatus::Running { .. } => 1,
        ServerStatus::Stopping => 3,
        ServerStatus::Stopped => 0,
        ServerStatus::Failed { .. } => 4,
    }
}

/// What to do after a configuration or root-directory change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestartDisposition {
    /// No server is running; the new settings apply on the next start.
    NotRunning,
    /// The running server already uses exactly this configuration.
    Unchanged,
    /// No transfer was active and the server was asked to restart.
    Restarted,
    /// Transfers are active; the restart is left to a manual action.
    Deferred { transfers: usize },
    /// The automatic restart failed; the error was logged.
    Failed,
}

/// Restart the running server with `config` when nothing is being transferred.
///
/// A configuration change while transfers are active is only saved: restarting
/// would abort the current downloads. The caller stores the new configuration
/// in `state.config` before calling this.
fn auto_restart_if_idle(
    controller: &Rc<RefCell<EmbeddedController>>,
    state: &mut UiState,
    config: &ServerConfig,
) -> RestartDisposition {
    let status = controller.borrow_mut().poll_status();
    // Starting counts as running; Stopping is a deliberate shutdown and must
    // not be turned back into a start.
    let running = matches!(
        status,
        ServerStatus::Starting | ServerStatus::Running { .. }
    );
    if !running {
        return RestartDisposition::NotRunning;
    }
    if state.running_config.as_ref() == Some(config) {
        return RestartDisposition::Unchanged;
    }
    if !state.transfers.is_empty() {
        return RestartDisposition::Deferred {
            transfers: state.transfers.len(),
        };
    }
    match controller.borrow_mut().restart(config.clone()) {
        Ok(()) => {
            state.running_config = Some(config.clone());
            state.running_root = Some(config.root_dir.clone());
            state.failed_message = None;
            // Suppress the transient Stopped log, exactly like the manual
            // restart path.
            state.last_status_key = status_key(&ServerStatus::Stopping);
            RestartDisposition::Restarted
        }
        Err(error) => {
            let message = state.text.restart_failed(&error.to_string());
            state.push_log(LogLevel::Error, message);
            RestartDisposition::Failed
        }
    }
}

fn sync_running_root(state: &mut UiState, status: &ServerStatus) {
    if matches!(status, ServerStatus::Stopped | ServerStatus::Failed { .. }) {
        state.running_root = None;
        state.running_config = None;
    }
}

fn log_status_transition(state: &mut UiState, status: &ServerStatus) {
    let key = status_key(status);
    if key == state.last_status_key {
        return;
    }
    match status {
        ServerStatus::Running { local_addrs } => {
            let root = state
                .running_root
                .as_deref()
                .unwrap_or(&state.config.root_dir);
            let root = display_root_path(root);
            let bind = summarize_addrs(local_addrs, state.text);
            let message = state.text.service_ready(&bind, &root);
            state.push_log(LogLevel::Info, message);
        }
        ServerStatus::Stopped => {
            if state.last_status_key != 0 {
                state.push_log(LogLevel::Info, state.text.service_stopped());
            }
        }
        ServerStatus::Failed { message } => {
            let message = state.text.service_error(message);
            state.push_log(LogLevel::Error, message);
        }
        ServerStatus::Starting | ServerStatus::Stopping => {}
    }
    state.last_status_key = key;
}

fn drain_events(events: &Arc<EventBus>, state: &mut UiState) {
    let new_events = events.drain_since(state.last_seq);
    if let Some(last) = new_events.last() {
        state.last_seq = last.seq;
    }
    for event in new_events {
        handle_event(state, event);
    }
}

fn handle_event(state: &mut UiState, event: TftpEvent) {
    match &event.kind {
        EventKind::Started => {
            state.transfers.insert(
                event.session_id,
                TransferState {
                    file: event.file.clone(),
                    peer: event.peer.to_string(),
                    bytes: 0,
                    total: event.total,
                    started: Instant::now(),
                },
            );
            let size = event
                .total
                .map_or_else(|| state.text.size_unknown().to_string(), format_bytes);
            let message = state
                .text
                .transfer_started(&event.file, &size, &event.peer.to_string());
            state.push_log(LogLevel::Info, message);
        }
        EventKind::Progress => {
            if let Some(transfer) = state.transfers.get_mut(&event.session_id) {
                transfer.bytes = event.bytes;
            }
        }
        EventKind::Completed => {
            // Log the completion even if the matching `Started` event was already
            // evicted from the ring: a finished transfer must always leave a
            // trace in the activity log.
            let transfer = state.transfers.remove(&event.session_id);
            let file = transfer
                .as_ref()
                .map_or_else(|| event.file.clone(), |transfer| transfer.file.clone());
            let elapsed = transfer
                .as_ref()
                .map(|transfer| transfer.started.elapsed().as_secs_f32());
            let message = state
                .text
                .transfer_completed(&file, &format_bytes(event.bytes), elapsed);
            state.push_log(LogLevel::Info, message);
        }
        EventKind::Failed { reason } => {
            let transfer = state.transfers.remove(&event.session_id);
            let file = transfer.map_or_else(|| event.file.clone(), |transfer| transfer.file);
            let message = state.text.transfer_failed(&file, reason);
            state.push_log(LogLevel::Warn, message);
        }
        EventKind::Rejected { reason } => {
            let message = state.text.request_rejected(&event.file, reason);
            state.push_log(LogLevel::Warn, message);
        }
    }
}

fn refresh_models(app: &AppWindow, state: &mut UiState) {
    let mut rows: Vec<TransferRow> = state
        .transfers
        .iter()
        .map(|(id, transfer)| {
            let progress = match transfer.total {
                Some(total) if total > 0 => (transfer.bytes as f32 / total as f32).clamp(0.0, 1.0),
                Some(_) => 1.0,
                None => 0.0,
            };
            let percent = if transfer.total.is_some() {
                format!("{:.1}%", f64::from(progress) * 100.0)
            } else {
                "—".to_string()
            };
            TransferRow {
                id: i32::try_from(*id).unwrap_or(i32::MAX),
                file: transfer.file.clone().into(),
                peer: transfer.peer.clone().into(),
                bytes: match transfer.total {
                    Some(total) => {
                        format!("{} / {}", format_bytes(transfer.bytes), format_bytes(total))
                    }
                    None => format_bytes(transfer.bytes),
                }
                .into(),
                percent: percent.into(),
                progress,
                status: state.text.transfer_in_progress().into(),
            }
        })
        .collect();
    rows.sort_by_key(|row| row.id);
    state.transfers_model.set_vec(rows);

    let total_bytes: u64 = state
        .transfers
        .values()
        .map(|transfer| transfer.bytes)
        .sum();
    let now = Instant::now();
    let elapsed = now.duration_since(state.throughput_sample.0).as_secs_f32();
    let delta = total_bytes.saturating_sub(state.throughput_sample.1);
    let dash = state.text.dash().to_string();
    let throughput = if state.transfers.is_empty() {
        dash
    } else if elapsed > 0.0 {
        format!("{}/s", format_bytes((delta as f32 / elapsed) as u64))
    } else {
        dash
    };
    state.throughput_sample = (now, total_bytes);

    app.set_transfers(ModelRc::from(state.transfers_model.clone()));
    if state.logs_dirty {
        state.rebuild_log_model();
        app.set_logs(ModelRc::from(state.log_model.clone()));
        state.logs_dirty = false;
    }
    app.set_log_total_count(i32::try_from(state.logs.len()).unwrap_or(i32::MAX));
    if state.refresh_recent_model() {
        app.set_recent_dirs(ModelRc::from(state.recent_model.clone()));
    }
    app.set_active_text(state.text.active_transfers(state.transfers.len()).into());
    app.set_throughput_text(throughput.into());
}

fn recent_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || display_path(path),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn same_path(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let value = bytes as f64;
    if value >= GIB {
        format!("{:.2} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.2} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("ip address")
    }

    /// English texts, the fallback language of the panel.
    fn en() -> Text {
        Text::new(Language::En)
    }

    /// Simplified Chinese texts.
    fn zh() -> Text {
        Text::new(Language::ZhCn)
    }

    fn test_state(config: ServerConfig) -> UiState {
        UiState::new(
            config,
            RecentDirs::empty_for_tests(),
            LastRoot::empty_for_tests(),
            Arc::new(StdMutex::new(Some(Vec::new()))),
            Appearance::for_tests(en()),
        )
    }

    fn is_selected(options: &[AddressChoice], address: &str) -> bool {
        options
            .iter()
            .find(|option| option.ip == ip(address))
            .is_some_and(|option| option.selected)
    }

    #[test]
    fn first_run_defaults_to_all_networks() {
        let config = gui_default_config();
        assert_eq!(
            config.bind_addresses,
            vec![IpAddr::V4(Ipv4Addr::UNSPECIFIED)]
        );
        assert_eq!(config.port, 69);
        assert!(!config.allow_upload);
        assert!(!config.overwrite);
        // The panel has no root directory before the operator selects one. The
        // core/daemon default (`tftp-root`) must stay untouched.
        assert_eq!(config.root_dir, PathBuf::new());
        assert_eq!(
            root_status_text(&config.root_dir, en()),
            "No root directory"
        );
        assert_eq!(display_root_or_empty(&config.root_dir), "");
        assert_eq!(ServerConfig::default().root_dir, PathBuf::from("tftp-root"));
    }

    #[test]
    fn missing_or_invalid_config_uses_the_panel_default_and_keeps_the_error() {
        let missing = InitialConfig::Missing;
        assert!(!missing.is_configured());
        assert!(missing.error().is_none());
        assert_eq!(missing.config(None).bind_addresses, vec![ip("0.0.0.0")]);
        assert_eq!(missing.config(None).root_dir, PathBuf::new());

        let invalid = InitialConfig::Invalid("bad toml".to_string());
        assert!(!invalid.is_configured());
        assert_eq!(invalid.error(), Some("bad toml"));
        assert_eq!(invalid.config(None).bind_addresses, vec![ip("0.0.0.0")]);
        assert_eq!(invalid.config(None).root_dir, PathBuf::new());
    }

    #[test]
    fn a_remembered_root_is_restored_when_no_config_file_exists() {
        let dir = tempfile::tempdir().expect("temp dir");
        let share = dir.path().join("share");
        std::fs::create_dir(&share).expect("share dir");
        // `LastRoot` stores the canonical path, which on Windows carries the
        // verbatim prefix; the panel keeps it and displays it without the prefix.
        let remembered = std::fs::canonicalize(&share).expect("canonical");
        let restored = InitialConfig::Missing.config(Some(&remembered));
        assert_eq!(restored.root_dir, remembered);
        // The restored root still shows its leaf name in the status bar.
        assert_eq!(root_status_text(&restored.root_dir, en()), "share");
    }

    #[test]
    fn a_remembered_root_that_no_longer_exists_is_ignored() {
        let dir = tempfile::tempdir().expect("temp dir");
        let gone = dir.path().join("gone");
        assert_eq!(
            InitialConfig::Missing.config(Some(&gone)).root_dir,
            PathBuf::new()
        );
        let file = dir.path().join("file.txt");
        std::fs::write(&file, b"x").expect("write file");
        assert_eq!(
            InitialConfig::Missing.config(Some(&file)).root_dir,
            PathBuf::new()
        );
    }

    #[test]
    fn a_loaded_config_root_wins_over_the_remembered_root() {
        let configured = tempfile::tempdir().expect("configured dir");
        let remembered = tempfile::tempdir().expect("remembered dir");
        let config = ServerConfig {
            root_dir: configured.path().to_path_buf(),
            ..ServerConfig::default()
        };
        let loaded = InitialConfig::Loaded(config.clone());
        assert_eq!(loaded.config(Some(remembered.path())), config);
    }

    #[test]
    fn an_empty_root_field_is_rejected_with_an_actionable_message() {
        assert_eq!(
            root_from_text("  \r\n", en()),
            Err("Choose a root directory first.".to_string())
        );
        assert_eq!(
            root_from_text("  C:/srv/tftp  ", en()),
            Ok(PathBuf::from("C:/srv/tftp"))
        );
    }

    #[test]
    fn a_loaded_config_keeps_its_addresses_for_the_first_start() {
        let config = ServerConfig {
            bind_addresses: vec![ip("127.0.0.1"), ip("10.0.0.7")],
            ..ServerConfig::default()
        };
        let loaded = InitialConfig::Loaded(config.clone());
        assert!(loaded.is_configured());
        assert!(loaded.error().is_none());
        assert_eq!(loaded.config(None), config);
    }

    #[test]
    fn choosing_all_networks_replaces_the_specific_addresses() {
        let mut options = build_address_choices(&[ip("127.0.0.1")], &[ip("10.0.0.7")], en());
        assert!(is_selected(&options, "127.0.0.1"));

        toggle_address_choice(&mut options, ip("0.0.0.0"));
        assert!(is_selected(&options, "0.0.0.0"));
        assert!(!is_selected(&options, "127.0.0.1"));
        assert!(!is_selected(&options, "10.0.0.7"));
        assert_eq!(options.iter().filter(|option| option.selected).count(), 1);
    }

    #[test]
    fn choosing_a_specific_address_clears_all_networks() {
        let mut options = build_address_choices(&[ip("0.0.0.0")], &[ip("10.0.0.7")], en());
        toggle_address_choice(&mut options, ip("10.0.0.7"));
        assert!(!is_selected(&options, "0.0.0.0"));
        assert!(is_selected(&options, "10.0.0.7"));

        // A second specific address joins the list instead of replacing it.
        toggle_address_choice(&mut options, ip("127.0.0.1"));
        assert!(is_selected(&options, "10.0.0.7"));
        assert!(is_selected(&options, "127.0.0.1"));
        assert_eq!(options.iter().filter(|option| option.selected).count(), 2);
    }

    #[test]
    fn all_networks_cannot_be_unchecked() {
        let mut options = build_address_choices(&[ip("0.0.0.0")], &[], en());
        toggle_address_choice(&mut options, ip("0.0.0.0"));
        assert!(is_selected(&options, "0.0.0.0"));
        assert_eq!(options.iter().filter(|option| option.selected).count(), 1);
    }

    #[test]
    fn clearing_the_last_address_falls_back_to_all_networks() {
        let mut options = build_address_choices(&[ip("127.0.0.1")], &[ip("10.0.0.7")], en());
        toggle_address_choice(&mut options, ip("127.0.0.1"));
        assert!(is_selected(&options, "0.0.0.0"));
        assert!(!is_selected(&options, "127.0.0.1"));
        assert_eq!(options.iter().filter(|option| option.selected).count(), 1);
    }

    #[test]
    fn toggling_an_unknown_address_is_ignored() {
        let mut options = build_address_choices(&[ip("0.0.0.0")], &[], en());
        toggle_address_choice(&mut options, ip("192.0.2.1"));
        assert!(is_selected(&options, "0.0.0.0"));
        assert_eq!(options.len(), 2);
    }

    #[test]
    fn auto_restart_runs_only_for_running_servers_and_idle_transfers() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("file.bin"), b"payload").expect("write file");
        let config = ServerConfig {
            root_dir: dir.path().to_path_buf(),
            port: 0,
            bind_addresses: vec![ip("127.0.0.1")],
            timeout_secs: 1,
            max_retries: 1,
            shutdown_grace_secs: 2,
            ..ServerConfig::default()
        };
        let mut state = test_state(config.clone());
        let controller = Rc::new(RefCell::new(EmbeddedController::new().expect("controller")));

        // A stopped server only stores the new configuration.
        assert_eq!(
            auto_restart_if_idle(&controller, &mut state, &config),
            RestartDisposition::NotRunning
        );

        controller
            .borrow_mut()
            .start(config.clone())
            .expect("start");
        state.running_config = Some(config.clone());
        assert_eq!(
            auto_restart_if_idle(&controller, &mut state, &config),
            RestartDisposition::Unchanged
        );

        // An active transfer defers the restart and keeps the old configuration.
        let mut changed = config.clone();
        changed.timeout_secs = 2;
        state.transfers.insert(
            1,
            TransferState {
                file: "file.bin".to_string(),
                peer: "127.0.0.1:1".to_string(),
                bytes: 0,
                total: None,
                started: Instant::now(),
            },
        );
        assert_eq!(
            auto_restart_if_idle(&controller, &mut state, &changed),
            RestartDisposition::Deferred { transfers: 1 }
        );
        assert_eq!(state.running_config.as_ref(), Some(&config));

        // With no transfer the same change restarts the server.
        state.transfers.clear();
        assert_eq!(
            auto_restart_if_idle(&controller, &mut state, &changed),
            RestartDisposition::Restarted
        );
        assert_eq!(state.running_config.as_ref(), Some(&changed));

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = controller.borrow_mut().poll_status();
            if matches!(status, ServerStatus::Running { .. }) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "automatic restart did not finish: {status:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }

        // A deliberate stop must not be turned back into a start by a change.
        assert!(controller.borrow_mut().stop());
        assert_eq!(
            auto_restart_if_idle(&controller, &mut state, &config),
            RestartDisposition::NotRunning
        );
        controller.borrow_mut().shutdown();
    }

    #[test]
    fn address_choices_always_offer_the_two_special_entries() {
        let options = build_address_choices(&[ip("127.0.0.1")], &[ip("192.168.1.5")], en());
        let labels: Vec<&str> = options.iter().map(|option| option.label.as_str()).collect();
        assert_eq!(labels[0], "All networks (0.0.0.0)");
        assert_eq!(labels[1], "Loopback (127.0.0.1)");
        assert!(labels.contains(&"192.168.1.5"));
        assert_eq!(options.iter().filter(|option| option.selected).count(), 1);
    }

    #[test]
    fn selected_addresses_missing_from_discovery_stay_visible() {
        let selected = vec![ip("::1"), ip("10.1.2.3")];
        let options = build_address_choices(&selected, &[], en());
        assert_eq!(options.len(), 4);
        assert!(options.iter().any(|option| option.ip == ip("::1")));
        assert_eq!(options.iter().filter(|option| option.selected).count(), 2);
    }

    #[test]
    fn duplicate_selected_addresses_are_not_repeated() {
        let options = build_address_choices(&[ip("127.0.0.1"), ip("127.0.0.1")], &[], en());
        assert_eq!(options.len(), 2);
        assert_eq!(options.iter().filter(|option| option.selected).count(), 1);
    }

    #[test]
    fn address_labels_cover_the_special_entries() {
        assert_eq!(address_label(ip("0.0.0.0"), en()), "All networks (0.0.0.0)");
        assert_eq!(address_label(ip("127.0.0.1"), en()), "Loopback (127.0.0.1)");
        assert_eq!(address_label(ip("::"), en()), "All IPv6 networks (::)");
        assert_eq!(address_label(ip("::1"), en()), "Loopback IPv6 (::1)");
        assert_eq!(address_label(ip("192.168.1.5"), en()), "192.168.1.5");
    }

    #[test]
    fn summary_is_short_for_many_addresses() {
        let options = build_address_choices(&[ip("0.0.0.0")], &[], en());
        assert_eq!(bind_summary_text(&options, en()), "All networks (0.0.0.0)");
        assert_eq!(bind_summary_text(&[], en()), "No address selected");

        let many =
            build_address_choices(&[ip("0.0.0.0"), ip("10.0.0.1"), ip("10.0.0.2")], &[], en());
        assert_eq!(
            bind_summary_text(&many, en()),
            "All networks (0.0.0.0), 10.0.0.1 and 3 addresses"
        );
    }

    #[test]
    fn status_bar_summarises_many_listeners() {
        let one: Vec<SocketAddr> = vec!["127.0.0.1:69".parse().expect("addr")];
        assert_eq!(summarize_addrs(&one, en()), "127.0.0.1:69");
        let four: Vec<SocketAddr> = (1..=4)
            .map(|last| format!("127.0.0.{last}:69").parse().expect("addr"))
            .collect();
        assert_eq!(summarize_addrs(&four, en()), "127.0.0.1:69 and 4 addresses");
        assert_eq!(summarize_addrs(&[], en()), "—");
    }

    #[test]
    fn status_bar_root_shows_the_leaf_directory_name() {
        assert_eq!(root_display_name(Path::new("tftp-root")), "tftp-root");
        assert_eq!(root_display_name(Path::new("C:/srv/tftp")), "tftp");
        assert_eq!(root_status_text(Path::new("C:/srv/tftp"), en()), "tftp");
        assert_eq!(root_status_text(Path::new(""), en()), "No root directory");
    }

    #[test]
    fn remembering_the_root_keeps_the_last_selected_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = ServerConfig {
            root_dir: dir.path().to_path_buf(),
            ..ServerConfig::default()
        };
        let mut state = test_state(config);
        state.remember_root();
        let expected = std::fs::canonicalize(dir.path()).expect("canonical");
        assert_eq!(state.last_root.path(), Some(expected.as_path()));

        // An unset root must not overwrite the remembered directory.
        state.config.root_dir = PathBuf::new();
        state.remember_root();
        assert_eq!(state.last_root.path(), Some(expected.as_path()));
    }

    #[test]
    fn storing_a_root_updates_config_recent_history_and_memory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let share = dir.path().join("share-b");
        std::fs::create_dir(&share).expect("share dir");
        let canonical = std::fs::canonicalize(&share).expect("canonical");

        let mut state = test_state(gui_default_config());
        let display = store_root(&mut state, share.clone());
        assert_eq!(state.config.root_dir, share);
        assert_eq!(display, display_path(&canonical));
        assert_eq!(state.recent.paths(), &[canonical.clone()][..]);
        assert_eq!(state.last_root.path(), Some(canonical.as_path()));
    }

    #[test]
    fn the_recent_menu_model_is_only_rebuilt_when_its_rows_change() {
        let dir = tempfile::tempdir().expect("temp dir");
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        std::fs::create_dir(&first).expect("first dir");
        std::fs::create_dir(&second).expect("second dir");
        let mut state = test_state(gui_default_config());

        // Repeated ticks over an empty history must not reset the model.
        assert!(!state.refresh_recent_model());
        assert!(!state.refresh_recent_model());

        assert!(state.recent.record(&first));
        assert!(state.refresh_recent_model());
        assert_eq!(state.recent_model.row_count(), 1);
        // The 200 ms refresh sees the same rows and leaves the Slint repeater
        // alone. Replacing the model here would destroy the hover state and
        // invalidate a click press that spans two ticks.
        assert!(!state.refresh_recent_model());
        assert!(!state.refresh_recent_model());

        // Starting or stopping the server only flips the "current" marker and
        // still has to reach the menu.
        let active = std::fs::canonicalize(&first).expect("canonical");
        state.running_root = Some(active);
        assert!(state.refresh_recent_model());
        assert!(
            state.recent_model.row_data(0).expect("first row").active,
            "the running root must be marked as current"
        );
        assert!(!state.refresh_recent_model());

        // Recording another directory changes the list itself.
        assert!(state.recent.record(&second));
        assert!(state.refresh_recent_model());
        assert_eq!(state.recent_model.row_count(), 2);
        assert!(!state.refresh_recent_model());

        // Clearing the history empties the model once, not on every tick.
        state.recent.clear();
        assert!(state.refresh_recent_model());
        assert_eq!(state.recent_model.row_count(), 0);
        assert!(!state.refresh_recent_model());
    }

    #[test]
    fn pending_dialogs_are_polled_with_their_target_and_snapshot() {
        let mut state = test_state(gui_default_config());
        let (sender, receiver) = std::sync::mpsc::channel();
        state.pending_pick = Some(PendingFolderPick {
            target: FolderPickTarget::Root,
            previous_hint: String::new(),
            receiver,
        });
        assert!(poll_folder_picker(&mut state).is_none());
        sender
            .send(Some(PathBuf::from("C:/share")))
            .expect("send pick");
        let outcome = poll_folder_picker(&mut state).expect("picker result");
        assert_eq!(outcome.target, FolderPickTarget::Root);
        assert_eq!(outcome.picked, Some(PathBuf::from("C:/share")));
        assert!(state.pending_pick.is_none());

        let (sender, receiver) = std::sync::mpsc::channel();
        state.log_export = Some(PendingLogExport {
            receiver,
            text: "snapshot".to_string(),
        });
        assert!(poll_log_export(&mut state).is_none());
        let path = PathBuf::from("C:/logs/tftpd-r-log.txt");
        sender
            .send(SaveChoice::Path(path.clone()))
            .expect("send export");
        assert_eq!(
            poll_log_export(&mut state),
            Some((SaveChoice::Path(path), "snapshot".to_string()))
        );
        assert!(state.log_export.is_none());
    }

    #[test]
    fn log_model_stays_bounded_and_keeps_newest_entries_first() {
        let mut state = test_state(gui_default_config());

        for index in 0..=MAX_LOG_ROWS {
            state.push_log(LogLevel::Info, format!("entry-{index}"));
        }

        assert_eq!(state.logs.len(), MAX_LOG_ROWS);
        state.rebuild_log_model();
        assert_eq!(state.log_model.row_count(), MAX_LOG_ROWS);
        let newest = state.log_model.row_data(0).expect("newest row");
        assert_eq!(newest.message.as_str(), format!("entry-{MAX_LOG_ROWS}"));
        assert_eq!(newest.level.as_str(), "INFO");
        assert_eq!(newest.time.len(), 8, "HH:MM:SS wall-clock label");
        assert_eq!(
            state
                .log_model
                .row_data(MAX_LOG_ROWS - 1)
                .expect("oldest retained row")
                .message
                .as_str(),
            "entry-1"
        );
    }

    #[test]
    fn log_filter_only_hides_rows_and_never_drops_entries() {
        let mut state = test_state(gui_default_config());
        state.push_log(LogLevel::Info, "info");
        state.push_log(LogLevel::Warn, "warn");
        state.push_log(LogLevel::Error, "error");
        assert_eq!(state.logs.len(), 3);

        state.log_filter = LogFilter::Warn;
        state.rebuild_log_model();
        assert_eq!(state.log_model.row_count(), 1);
        let row = state.log_model.row_data(0).expect("filtered row");
        assert_eq!(row.level.as_str(), "WARN");
        assert_eq!(row.message.as_str(), "warn");
        assert_eq!(state.logs.len(), 3, "the filter must not drop entries");

        state.log_filter = LogFilter::Error;
        state.rebuild_log_model();
        assert_eq!(state.log_model.row_count(), 1);
        assert_eq!(
            state
                .log_model
                .row_data(0)
                .expect("error row")
                .message
                .as_str(),
            "error"
        );

        state.log_filter = LogFilter::All;
        state.rebuild_log_model();
        assert_eq!(state.log_model.row_count(), 3);
        assert_eq!(
            state
                .log_model
                .row_data(0)
                .expect("newest row")
                .message
                .as_str(),
            "error",
            "rows stay newest-first"
        );
    }

    #[test]
    fn the_product_icon_and_brand_mark_share_one_geometry() {
        assert_eq!(
            APP_ICON_RGBA.len(),
            (APP_ICON_SIZE * APP_ICON_SIZE * 4) as usize,
            "app-icon-128.rgba must hold one RGBA quad per pixel"
        );
        let pixel = |x: u32, y: u32| {
            let offset = ((y * APP_ICON_SIZE + x) * 4) as usize;
            &APP_ICON_RGBA[offset..offset + 4]
        };
        // Rounded square: the corners are transparent, the tile is #263238
        // (the sheen barely moves it at mid height) and the middle of the upper
        // server rack is the #fafafa glyph.
        assert_eq!(pixel(0, 0)[3], 0);
        assert_eq!(pixel(APP_ICON_SIZE - 1, APP_ICON_SIZE - 1)[3], 0);
        let tile = pixel(8, APP_ICON_SIZE / 2);
        assert_eq!(tile[3], 255);
        for (channel, expected) in tile[..3].iter().zip([0x26u8, 0x32, 0x38]) {
            assert!(
                channel.abs_diff(expected) <= 3,
                "tile pixel {tile:?} is not close to #263238"
            );
        }
        assert_eq!(pixel(APP_ICON_SIZE / 2, 44), &[0xfa, 0xfa, 0xfa, 255]);

        // The `dns` path fills 18 of Material's 24-unit viewbox and the master
        // artwork scales that viewbox to 192/256 of the tile. `BrandMark` must
        // therefore size the viewbox at 75 % of the mark; sizing it at the
        // visible-ink ratio (56.25 %) — the earlier value — shrank the glyph to
        // 42 % and made the title-bar/About badge look like a different logo
        // from the window, task-bar and PE icon.
        let widgets = include_str!("../ui/widgets.slint");
        assert!(
            widgets.contains("icon-size: root.mark-size * 0.75;"),
            "BrandMark must map the 24-unit Material viewbox to 75 % of the tile"
        );
        assert!(
            widgets.contains("background: Theme.brand-sheen;"),
            "BrandMark must paint the same sheen as the exported icon"
        );
        // The shipped raster must keep the same ink ratio (144/256 of the
        // tile), so the vector badge and the bitmaps cannot drift apart again.
        let mut bounds: Option<(u32, u32, u32, u32)> = None;
        for y in 0..APP_ICON_SIZE {
            for x in 0..APP_ICON_SIZE {
                let pixel = pixel(x, y);
                let opaque_glyph =
                    pixel[3] > 200 && pixel[..3].iter().all(|channel| *channel > 200);
                if !opaque_glyph {
                    continue;
                }
                bounds = Some(match bounds {
                    None => (x, y, x, y),
                    Some((min_x, min_y, max_x, max_y)) => {
                        (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
                    }
                });
            }
        }
        let (min_x, min_y, max_x, max_y) = bounds.expect("the icon must contain glyph pixels");
        let ink = (max_x - min_x + 1).max(max_y - min_y + 1) as f32;
        let expected = APP_ICON_SIZE as f32 * 0.75 * 18.0 / 24.0;
        assert!(
            (ink - expected).abs() <= 2.0,
            "shipped icon ink is {ink} px, but the brand mark implies {expected} px"
        );
    }

    #[test]
    fn the_repository_link_uses_a_fixed_https_address_and_a_shell_free_handler() {
        let url = env!("CARGO_PKG_REPOSITORY");
        assert_eq!(url, "https://github.com/greyskysoul/tftpd-r");
        assert!(url.starts_with("https://"));

        // The link handler must receive the URL as one argument and must not
        // involve a shell, so the fixed address cannot turn into a command.
        let command = repository_command(url);
        let program = command.get_program().to_string_lossy().into_owned();
        let args: Vec<String> = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        #[cfg(target_os = "windows")]
        {
            assert!(
                program.ends_with("rundll32.exe"),
                "unexpected browser handler {program}"
            );
            assert_eq!(
                args,
                vec!["url.dll,FileProtocolHandler".to_owned(), url.to_owned()]
            );
        }
        #[cfg(not(target_os = "windows"))]
        {
            assert_eq!(program, "xdg-open");
            assert_eq!(args, vec![url.to_owned()]);
        }
    }

    #[test]
    fn address_labels_and_summaries_follow_the_startup_language() {
        // The same helpers drive the picker, the settings summary and the status
        // bar, so a Chinese startup must not leak English labels (or the ASCII
        // comma that reads wrong in Chinese).
        assert_eq!(address_label(ip("0.0.0.0"), zh()), "全部网络 (0.0.0.0)");
        assert_eq!(address_label(ip("127.0.0.1"), zh()), "本地回环 (127.0.0.1)");
        assert_eq!(address_label(ip("::"), zh()), "全部 IPv6 网络 (::)");
        assert_eq!(address_label(ip("::1"), zh()), "本地回环 IPv6 (::1)");

        let options = build_address_choices(&[ip("0.0.0.0")], &[], zh());
        assert_eq!(bind_summary_text(&options, zh()), "全部网络 (0.0.0.0)");
        assert_eq!(bind_summary_text(&[], zh()), "未选择地址");

        let many =
            build_address_choices(&[ip("0.0.0.0"), ip("10.0.0.1"), ip("10.0.0.2")], &[], zh());
        assert_eq!(
            bind_summary_text(&many, zh()),
            "全部网络 (0.0.0.0)、10.0.0.1 等 3 个地址"
        );

        let four: Vec<SocketAddr> = (1..=4)
            .map(|last| format!("127.0.0.{last}:69").parse().expect("addr"))
            .collect();
        assert_eq!(summarize_addrs(&four, zh()), "127.0.0.1:69 等 4 个地址");
        assert_eq!(root_status_text(Path::new(""), zh()), "未指定根目录");
    }

    #[test]
    fn the_theme_mode_decides_the_resolved_palette() {
        let mut state = test_state(gui_default_config());
        // Following the system with a dark system yields a dark panel.
        state.theme_mode = ThemeMode::System;
        state.system_dark = true;
        assert!(state.is_dark());
        state.system_dark = false;
        assert!(!state.is_dark());

        // Explicit choices ignore the system value.
        state.theme_mode = ThemeMode::Light;
        state.system_dark = true;
        assert!(!state.is_dark());
        state.theme_mode = ThemeMode::Dark;
        state.system_dark = false;
        assert!(state.is_dark());

        // The default preference follows the system.
        assert_eq!(state.prefs.theme_mode(), ThemeMode::System);
    }

    #[test]
    fn the_modal_scrim_is_a_neutral_black_overlay() {
        // Slint parses an eight-digit hex literal as `#rrggbbaa`, not ARGB, so
        // the old backdrop (`dark ? #99000000 : #66111827`) painted a
        // translucent maroon wash in light mode and a fully transparent overlay
        // in dark mode. The token must stay an explicit black `rgba()` so the
        // channels are neutral in both schemes and the alpha is unambiguous.
        let theme = include_str!("../ui/theme.slint");
        let scrim = theme
            .lines()
            .find(|line| line.trim_start().starts_with("out property <color> scrim:"))
            .expect("theme.slint must define the scrim token");

        let branches: Vec<&str> = scrim
            .split_once('?')
            .expect("the scrim token must pick a dark and a light value")
            .1
            .trim_end_matches(';')
            .split(" : ")
            .map(str::trim)
            .collect();
        assert_eq!(branches.len(), 2, "unexpected scrim expression: {scrim}");
        for branch in branches {
            let alpha = branch
                .strip_prefix("rgba(0, 0, 0,")
                .and_then(|rest| rest.strip_suffix(')'))
                .unwrap_or_else(|| panic!("the scrim must be neutral black: {branch}"));
            let alpha: f32 = alpha
                .trim()
                .parse()
                .expect("the scrim alpha must be a plain float");
            assert!(
                (0.30..=0.85).contains(&alpha),
                "the scrim must visibly dim the panel, got alpha {alpha}"
            );
        }
    }

    #[test]
    fn the_startup_language_of_the_state_is_used_for_new_log_lines() {
        let mut state = test_state(gui_default_config());
        assert_eq!(state.text, en());
        let mut chinese = test_state(gui_default_config());
        chinese.text = zh();
        let message = chinese.text.first_start();
        assert_eq!(message, "首次启动：请选择根目录和绑定地址，然后保存设置。");
        chinese.push_log(LogLevel::Info, message);
        assert_eq!(
            chinese.logs.front().expect("startup line").message,
            "首次启动：请选择根目录和绑定地址，然后保存设置。"
        );

        // English is the default for an unsupported locale.
        state.push_log(LogLevel::Info, state.text.size_unknown());
        assert_eq!(
            state.logs.front().expect("english line").message,
            "unknown size"
        );
    }

    #[test]
    fn launch_flags_choose_the_startup_path() {
        let args = |list: &[&str]| -> Vec<String> {
            list.iter().map(|value| (*value).to_string()).collect()
        };
        assert_eq!(launch_mode(&args(&[])), LaunchMode::Normal);
        assert_eq!(launch_mode(&args(&["tftpd-r"])), LaunchMode::Normal);
        assert_eq!(
            launch_mode(&args(&["tftpd-r", "--smoke-test"])),
            LaunchMode::Normal
        );
        assert_eq!(
            launch_mode(&args(&["tftpd-r", "--new-instance"])),
            LaunchMode::NewInstance
        );
        assert_eq!(
            launch_mode(&args(&["tftpd-r", "--activate-only"])),
            LaunchMode::ActivateOnly
        );
        // Activation is the more specific request and must not be turned into a
        // normal launch by a second flag.
        assert_eq!(
            launch_mode(&args(&["tftpd-r", "--new-instance", "--activate-only"])),
            LaunchMode::ActivateOnly
        );
        // Only the exact long options are recognised.
        assert_eq!(
            launch_mode(&args(&["tftpd-r", "--new_instance"])),
            LaunchMode::Normal
        );
    }

    #[test]
    fn a_running_panel_puts_the_launch_into_the_prompt_state() {
        let dir = tempfile::tempdir().expect("temp dir");
        let owner = Owner::register(dir.path());
        let detected = Owner::detect(dir.path(), single_instance::now_millis())
            .expect("the registered panel must be found");
        assert_eq!(detected.token, owner.token());

        let mut state = test_state(gui_default_config());
        state.single_instance_prompt = Some(SingleInstancePrompt {
            background: detected,
            request: None,
        });
        let prompt = state
            .single_instance_prompt
            .as_ref()
            .expect("the prompt stays open until the operator decides");
        assert_eq!(prompt.background.pid, std::process::id());
        assert!(prompt.request.is_none());
        // "Cancel" clears the prompt without registering a lock; nothing else
        // in the state depends on it.
        state.single_instance_prompt = None;
        assert!(state.single_instance_prompt.is_none());
    }

    #[test]
    fn the_single_instance_prompt_offers_three_translated_choices() {
        // The prompt is the only UI of a second launch, so its wording must come
        // from the catalogue rather than raw English literals.
        let source = include_str!("../ui/single-instance.slint");
        for expected in [
            "@tr(\"TftpdR is already running\")",
            "@tr(\"Cancel\")",
            "@tr(\"New instance\")",
            "@tr(\"Show program window\")",
        ] {
            assert!(
                source.contains(expected),
                "the prompt must use the translated string {expected}"
            );
        }
        // The three callbacks are what the second launch actually does.
        for callback in [
            "callback activate-requested();",
            "callback new-instance-requested();",
            "callback cancel-requested();",
        ] {
            assert!(source.contains(callback), "missing {callback}");
        }
    }

    #[test]
    fn the_single_instance_prompt_is_its_own_small_window() {
        // The second launch must not flash (or build into the visual tree of)
        // the full panel: the prompt is a Window-rooted component of its own,
        // and the main window only contains the close prompt overlay.
        let prompt = include_str!("../ui/single-instance.slint");
        assert!(prompt.contains("export component InstancePromptWindow inherits Window"));
        assert!(!prompt.contains("Theme.scrim"));
        let app = include_str!("../ui/app.slint");
        assert!(!app.contains("instance-prompt-open"));
        assert!(app.contains("import { InstancePromptWindow } from \"single-instance.slint\""));
    }

    #[test]
    fn the_tray_icon_offers_show_and_quit() {
        let source = include_str!("../ui/tray.slint");
        assert!(source.contains("export component TrayIcon inherits SystemTrayIcon"));
        assert!(source.contains("@tr(\"Show program window\")"));
        assert!(source.contains("@tr(\"Quit\")"));
        assert!(source.contains("callback show-requested();"));
        assert!(source.contains("callback quit-requested();"));
        // Left-clicking the icon raises the window; the menu's first entry does
        // the same and the separator keeps Quit away from it.
        assert!(source.contains("clicked => { root.show-requested(); }"));
        assert!(source.contains("MenuSeparator { }"));
    }

    #[test]
    fn the_close_prompt_offers_exit_background_and_cancel() {
        let source = include_str!("../ui/close-prompt.slint");
        for expected in [
            "@tr(\"Close TftpdR?\")",
            "@tr(\"Close and exit\")",
            "@tr(\"Run in background\")",
            "@tr(\"Cancel\")",
        ] {
            assert!(
                source.contains(expected),
                "the close prompt must use the translated string {expected}"
            );
        }
        for callback in [
            "callback close-and-exit-requested();",
            "callback background-requested();",
            "callback cancel-requested();",
        ] {
            assert!(source.contains(callback), "missing {callback}");
        }
        // The background choice is only offered when a tray backend exists.
        assert!(source.contains("if root.background-available : SmallButton"));
    }

    #[test]
    fn only_a_starting_or_running_server_asks_before_closing() {
        assert!(server_is_active(&ServerStatus::Starting));
        assert!(server_is_active(&ServerStatus::Running {
            local_addrs: Vec::new(),
        }));
        assert!(!server_is_active(&ServerStatus::Stopping));
        assert!(!server_is_active(&ServerStatus::Stopped));
        assert!(!server_is_active(&ServerStatus::Failed {
            message: "boom".to_string(),
        }));
    }
}

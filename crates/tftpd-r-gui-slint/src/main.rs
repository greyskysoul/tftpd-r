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
mod log_export;
mod logging;
mod netif;
mod recent;
mod user_state;

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use chrono::Local;
use slint::{Model, ModelRc, Timer, TimerMode, VecModel};
use tftpd_r_core::config::display_path;
use tftpd_r_core::events::{EventBus, EventKind, SessionId, TftpEvent};
use tftpd_r_core::server::ServerStatus;
use tftpd_r_core::ServerConfig;

use controller::EmbeddedController;
use folder_picker::FolderPicker;
use log_export::SaveChoice;
use logging::{
    export_file_name, format_export, write_export, LogEntry, LogFilter, LogLevel, MAX_LOG_ROWS,
};
use recent::RecentDirs;
use user_state::{fallback_state_dir, LastRoot, SetupState};

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

struct UiState {
    last_seq: u64,
    last_status_key: i32,
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

impl UiState {
    fn new(
        config: ServerConfig,
        recent: RecentDirs,
        last_root: LastRoot,
        discovered: Arc<StdMutex<Option<Vec<IpAddr>>>>,
    ) -> Self {
        Self {
            last_seq: 0,
            last_status_key: 0,
            logs: VecDeque::new(),
            log_filter: LogFilter::All,
            log_model: Rc::new(VecModel::default()),
            logs_dirty: false,
            transfers_model: Rc::new(VecModel::default()),
            transfers: HashMap::new(),
            recent,
            recent_model: Rc::new(VecModel::default()),
            last_root,
            config,
            running_root: None,
            running_config: None,
            failed_message: None,
            throughput_sample: (Instant::now(), 0),
            address_options: Vec::new(),
            discovered,
            discovery_applied: false,
            folder_picker: FolderPicker::new(),
            pending_pick: None,
            picker_prewarmed: false,
            log_export: None,
        }
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

    fn refresh_recent_model(&mut self) {
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
        self.recent_model.set_vec(rows);
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
                Ok(true) => self.push_log(
                    LogLevel::Info,
                    format!(
                        "已记住根目录：{}，下次启动将自动恢复。",
                        display_root_path(&root)
                    ),
                ),
                Ok(false) => {}
                Err(error) => self.push_log(
                    LogLevel::Warn,
                    format!("无法记住根目录：{error}；下次启动不会自动恢复。"),
                ),
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
        self.address_options = build_address_choices(selected, discovered);
    }

    fn bind_summary(&self) -> String {
        bind_summary_text(&self.address_options)
    }
}

/// Build the picker entries: the two mandated special addresses first, then the
/// discovered interface addresses, then any selected address that discovery did
/// not report (so a hand-written configuration is never silently dropped).
fn build_address_choices(selected: &[IpAddr], discovered: &[IpAddr]) -> Vec<AddressChoice> {
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
            label: address_label(ip),
            selected: selected.contains(&ip),
        })
        .collect()
}

fn bind_summary_text(options: &[AddressChoice]) -> String {
    let selected: Vec<&AddressChoice> = options.iter().filter(|option| option.selected).collect();
    match selected.len() {
        0 => "未选择地址".to_string(),
        1 | 2 => selected
            .iter()
            .map(|option| option.label.clone())
            .collect::<Vec<_>>()
            .join("、"),
        count => format!(
            "{}、{} 等 {count} 个地址",
            selected[0].label, selected[1].label
        ),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|argument| argument == "--help" || argument == "-h") {
        println!("tftpd-r-gui {}", env!("CARGO_PKG_VERSION"));
        println!();
        println!("TftpdR native Slint panel for the embedded TFTP server (read-only RRQ).");
        println!();
        println!("Usage: tftpd-r-gui [OPTIONS]");
        println!();
        println!("Options:");
        println!("  -h, --help        Print this help and exit");
        println!(
            "      --smoke-test  Create the window, run a few UI ticks and exit (development)"
        );
        return Ok(());
    }
    let smoke_test = std::env::args().any(|argument| argument == "--smoke-test");
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

    let app = AppWindow::new()?;
    app.set_app_icon(app_icon());
    app.set_app_version(env!("CARGO_PKG_VERSION").into());

    let mut state = UiState::new(initial_config, recent, last_root, discovered);
    app.set_log_filter(state.log_filter.key().into());
    apply_config_to_settings(&app, &mut state);

    // First start without a usable `config.toml`: open the settings dialog so
    // the operator can fill in the root directory and bind addresses before the
    // first start. The flag is per user, so this happens once; `--smoke-test` and
    // the screenshot previews are development runs and must not consume it.
    let preview = std::env::var("TFTPD_R_GUI_PREVIEW").ok();
    let mut setup = SetupState::load();
    let show_setup =
        preview.is_none() && !smoke_test && !initial.is_configured() && setup.should_show_setup();
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
    state.push_log(
        LogLevel::Info,
        format!(
            "TftpdR {} 面板已启动：只读 RRQ，上传已禁用。",
            env!("CARGO_PKG_VERSION")
        ),
    );
    if let Some(dir) = fallback_state_dir() {
        state.push_log(
            LogLevel::Warn,
            format!(
                "用户配置目录不可写；状态文件改用便携目录：{}。",
                display_path(dir)
            ),
        );
    }
    if let Some(root) = restored_root {
        state.push_log(LogLevel::Info, format!("已恢复上次使用的根目录：{root}。"));
    }
    if let Some(stale) = stale_root {
        state.push_log(
            LogLevel::Warn,
            format!(
                "上次使用的根目录已不可用，已忽略：{}。",
                display_path(Path::new(&stale))
            ),
        );
    }
    if initial.is_configured() {
        state.push_log(LogLevel::Info, "已加载 config.toml。");
        if remembered_root.is_some() {
            state.push_log(
                LogLevel::Info,
                "config.toml 已提供根目录，因此未使用上次使用记录。",
            );
        }
    }
    if let Some(error) = initial.error() {
        state.push_log(
            LogLevel::Warn,
            format!("config.toml 不可用：{error}；已使用面板默认值。"),
        );
    }
    if show_setup {
        state.push_log(
            LogLevel::Info,
            "首次启动：请选择根目录和绑定地址，然后保存设置。",
        );
    }
    let state = Rc::new(RefCell::new(state));
    let controller = Rc::new(RefCell::new(EmbeddedController::new()?));

    apply_status(&app, &ServerStatus::Stopped, &state.borrow());
    install_callbacks(&app, &controller, &state);

    let timer = Timer::default();
    {
        let weak = app.as_weak();
        let controller = controller.clone();
        let state = state.clone();
        timer.start(TimerMode::Repeated, UI_TICK, move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let events = controller.borrow().events();
            let status = controller.borrow_mut().poll_status();
            let restart_error = controller.borrow_mut().take_error();
            let mut state = state.borrow_mut();
            drain_events(&events, &mut state);
            if let Some(error) = restart_error {
                state.failed_message = Some(error.clone());
                state.push_log(LogLevel::Error, format!("自动重启失败：{error}"));
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
        quit_timer.start(TimerMode::SingleShot, Duration::from_millis(1500), || {
            let _ = slint::quit_event_loop();
        });
    }

    let result = app.run();
    timer.stop();
    quit_timer.stop();
    // Stop the pre-warmed picker helper (including an open dialog) before the
    // process exits; otherwise PowerShell would outlive the panel.
    state.borrow().folder_picker.shutdown();
    controller.borrow_mut().shutdown();
    result?;
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
        Err(error) => InitialConfig::Invalid(format!("无法读取 config.toml：{error}")),
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
            // configured root when the dialog is saved, so 取消 still restores
            // the value that is currently in effect.
            let _ = state.recent.record(&path);
            app.set_settings_root(display.clone().into());
            // The log is behind the modal dialog, so repeat the "not saved yet"
            // hint in the form itself. Picking a directory only edits the form;
            // 保存设置 (or the recent-root menu's 选择…) applies it.
            app.set_settings_hint("已选择目录，点击“保存设置”后生效。".into());
            state.push_log(
                LogLevel::Info,
                format!("已选择根目录：{display}。点击“保存设置”后生效。"),
            );
        }
        Some(path) => {
            state.push_log(
                LogLevel::Error,
                format!("所选路径不是目录：{}。", display_path(&path)),
            );
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
            state.push_log(
                LogLevel::Error,
                format!("无法切换根目录：{} 不存在或不是目录。", display_path(&path)),
            );
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
/// idle server. Shared by the recent-root list and its 选择… button.
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
        RestartDisposition::Restarted => state.push_log(
            LogLevel::Info,
            format!("根目录已切换为 {root_text}；当前无进行中的传输，正在自动重启服务。"),
        ),
        RestartDisposition::Deferred { transfers } => state.push_log(
            LogLevel::Warn,
            format!(
                "根目录已切换为 {root_text}；有 {transfers} 个进行中的传输，未自动重启，稍后请手动重启。"
            ),
        ),
        RestartDisposition::Failed => state.push_log(
            LogLevel::Error,
            format!("根目录已切换为 {root_text}；自动重启失败，请手动重启。"),
        ),
        RestartDisposition::Unchanged => {
            state.push_log(LogLevel::Info, format!("根目录已经是 {root_text}，无需重启。"));
        }
        RestartDisposition::NotRunning => {
            state.push_log(
                LogLevel::Info,
                format!("根目录已设置为 {root_text}；将在下次启动时生效。"),
            );
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
            Ok(()) => state.push_log(
                LogLevel::Info,
                format!("运行日志已导出：{}。", display_path(&path)),
            ),
            Err(error) => state.push_log(LogLevel::Error, format!("导出运行日志失败：{error}。")),
        },
        SaveChoice::Cancelled => {}
        SaveChoice::Unavailable => state.push_log(
            LogLevel::Error,
            "导出运行日志失败：未找到可用的保存对话框（需要 PowerShell、zenity 或 kdialog）。",
        ),
    }
    refresh_models(app, state);
}

fn install_callbacks(
    app: &AppWindow,
    controller: &Rc<RefCell<EmbeddedController>>,
    state: &Rc<RefCell<UiState>>,
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
            let config = match read_config(&app, &state.config) {
                Ok(config) => config,
                Err(message) => {
                    app.set_settings_error(message.clone().into());
                    state.push_log(LogLevel::Error, format!("设置无效：{message}"));
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
                    let verb = if running { "重启" } else { "启动" };
                    let bind = state.config.bind_summary();
                    let root = display_root_path(&state.config.root_dir);
                    state.push_log(
                        LogLevel::Info,
                        format!("正在{verb}服务：监听 {bind}，根目录：{root}。"),
                    );
                    app.set_settings_error("".into());
                    app.set_settings_hint("".into());
                }
                Err(error) => {
                    let message = error.to_string();
                    state.failed_message = Some(message.clone());
                    state.push_log(
                        LogLevel::Error,
                        format!(
                            "无法{}服务：{message}",
                            if running { "重启" } else { "启动" }
                        ),
                    );
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
                state.push_log(LogLevel::Info, "正在停止服务；活动传输会按宽限期取消。");
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
            match read_config(&app, &state.config) {
                Ok(config) => {
                    state.config = config.clone();
                    state.remember_root();
                    let root = state.config.root_dir.clone();
                    let _ = state.recent.record(&root);
                    match auto_restart_if_idle(&controller, &mut state, &config) {
                        RestartDisposition::Restarted => state.push_log(
                            LogLevel::Info,
                            "设置已保存：当前无进行中的传输，正在自动重启服务。",
                        ),
                        RestartDisposition::Deferred { transfers } => state.push_log(
                            LogLevel::Warn,
                            format!(
                                "设置已保存：有 {transfers} 个进行中的传输，未自动重启，稍后请手动重启。"
                            ),
                        ),
                        RestartDisposition::Failed => state.push_log(
                            LogLevel::Error,
                            "设置已保存，但自动重启失败；请手动重启。",
                        ),
                        RestartDisposition::NotRunning => {
                            state.push_log(LogLevel::Info, "设置已保存；将在下次启动时生效。");
                        }
                        RestartDisposition::Unchanged => {
                            state.push_log(LogLevel::Info, "设置未发生变化。");
                        }
                    }
                    app.set_settings_error("".into());
                    app.set_settings_hint("".into());
                    app.set_settings_open(false);
                }
                Err(message) => {
                    app.set_settings_error(message.clone().into());
                    state.push_log(LogLevel::Error, format!("设置无效：{message}"));
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
            app.set_settings_hint("正在打开文件夹选择器…".into());
        });
    }
    // Native folder picker opened from the recent-root menu. Unlike the form
    // field above, the chosen directory is applied immediately (and restarts an
    // idle running server) without passing through 保存设置.
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
            let text = format_export(&state.logs, env!("CARGO_PKG_VERSION"), now);
            state.log_export = Some(PendingLogExport {
                receiver: log_export::save_log_async(export_file_name(now)),
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
                state.push_log(
                    LogLevel::Error,
                    format!("无法切换根目录：{} 不存在或不是目录。", display_path(&path)),
                );
                refresh_models(&app, &mut state);
                return;
            }
            switch_root(&app, &controller, &mut state, path);
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
            state.push_log(LogLevel::Info, "最近使用的根目录已清空。");
            refresh_models(&app, &mut state);
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
    {
        app.on_close_requested(|| {
            let _ = slint::quit_event_loop();
        });
    }
}

fn read_config(app: &AppWindow, fallback: &ServerConfig) -> Result<ServerConfig, String> {
    let port_text = app.get_settings_port().trim().to_string();
    let port: u16 = port_text
        .parse()
        .map_err(|_| format!("端口“{port_text}”无效，应为 1–65535。"))?;
    if port == 0 {
        return Err("端口必须在 1 到 65535 之间（0 仅用于测试）。".to_string());
    }
    let mut bind_addresses = Vec::new();
    for option in app.get_settings_address_options().iter() {
        if !option.selected {
            continue;
        }
        let address: IpAddr = option
            .address
            .parse()
            .map_err(|_| format!("绑定地址“{}”无效。", option.address))?;
        if !bind_addresses.contains(&address) {
            bind_addresses.push(address);
        }
    }
    if bind_addresses.is_empty() {
        return Err("请至少选择一个绑定地址。".to_string());
    }
    let root_dir = root_from_text(&app.get_settings_root())?;
    let block_size: u16 = parse_setting(&app.get_settings_block_size(), "最大块大小")?;
    let timeout_secs: u64 = parse_setting(&app.get_settings_timeout(), "超时")?;
    let max_sessions: usize = parse_setting(&app.get_settings_max_sessions(), "最大并发会话")?;
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
        .map_err(|error| format!("配置校验失败：{error}"))?;
    Ok(config)
}

/// Root directory from the settings field.
///
/// An empty field means the operator has not selected a root directory at all;
/// starting the server must ask for one instead of trying to resolve an empty
/// path (or a dummy placeholder directory).
fn root_from_text(text: &str) -> Result<PathBuf, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("请先选择根目录。".to_string());
    }
    Ok(PathBuf::from(text))
}

fn parse_setting<T: std::str::FromStr>(text: &str, label: &str) -> Result<T, String> {
    let text = text.trim();
    text.parse::<T>()
        .map_err(|_| format!("{label}“{text}”不是有效的数字。"))
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

fn address_label(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(ip) if ip.is_unspecified() => "全部网络 (0.0.0.0)".to_string(),
        IpAddr::V4(ip) if ip == Ipv4Addr::LOCALHOST => "本地回环 (127.0.0.1)".to_string(),
        IpAddr::V6(ip) if ip.is_unspecified() => "全部 IPv6 网络 (::)".to_string(),
        IpAddr::V6(ip) if ip == Ipv6Addr::LOCALHOST => "本地回环 IPv6 (::1)".to_string(),
        other => other.to_string(),
    }
}

fn apply_status(app: &AppWindow, status: &ServerStatus, state: &UiState) {
    let (text, kind, detail, primary_enabled, primary_is_restart, stop_enabled) = match status {
        ServerStatus::Starting => ("启动中".to_string(), 2, String::new(), false, false, true),
        ServerStatus::Running { .. } => ("运行中".to_string(), 1, String::new(), true, true, true),
        ServerStatus::Stopping => ("停止中".to_string(), 3, String::new(), false, false, false),
        ServerStatus::Stopped => match &state.failed_message {
            Some(message) => (
                "启动失败".to_string(),
                4,
                message.clone(),
                true,
                false,
                false,
            ),
            None => ("已停止".to_string(), 0, String::new(), true, false, false),
        },
        ServerStatus::Failed { message } => (
            "服务异常".to_string(),
            4,
            message.clone(),
            true,
            false,
            false,
        ),
    };
    let bind = match status {
        ServerStatus::Running { local_addrs } => summarize_addrs(local_addrs),
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
            root_status_text(path)
        }
        _ => root_status_text(&state.config.root_dir),
    };
    app.set_status_text(text.into());
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
/// "未指定根目录" instead of a dummy path when nothing is selected.
fn root_status_text(path: &Path) -> String {
    if root_is_set(path) {
        root_display_name(path)
    } else {
        "未指定根目录".to_string()
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
fn summarize_addrs(addrs: &[SocketAddr]) -> String {
    match addrs.len() {
        0 => "—".to_string(),
        1..=3 => addrs
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", "),
        count => format!("{} 等 {count} 个地址", addrs[0]),
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
            state.push_log(LogLevel::Error, format!("自动重启失败：{error}。"));
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
            state.push_log(
                LogLevel::Info,
                format!(
                    "服务已就绪：监听 {}，根目录：{root}。",
                    summarize_addrs(local_addrs)
                ),
            );
        }
        ServerStatus::Stopped => {
            if state.last_status_key != 0 {
                state.push_log(LogLevel::Info, "服务已停止。");
            }
        }
        ServerStatus::Failed { message } => {
            state.push_log(LogLevel::Error, format!("服务异常：{message}。"));
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
                .map_or_else(|| "大小未知".to_string(), format_bytes);
            state.push_log(
                LogLevel::Info,
                format!("开始传输：{}（{size}），来自 {}。", event.file, event.peer),
            );
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
            let elapsed = transfer.as_ref().map(|transfer| transfer.started.elapsed());
            let duration = elapsed.map_or_else(String::new, |elapsed| {
                format!("，耗时 {:.1} 秒", elapsed.as_secs_f32())
            });
            state.push_log(
                LogLevel::Info,
                format!(
                    "传输完成：{file}（{}{duration}）。",
                    format_bytes(event.bytes)
                ),
            );
        }
        EventKind::Failed { reason } => {
            let transfer = state.transfers.remove(&event.session_id);
            let file = transfer.map_or_else(|| event.file.clone(), |transfer| transfer.file);
            state.push_log(LogLevel::Warn, format!("传输失败：{file}（{reason}）。"));
        }
        EventKind::Rejected { reason } => {
            state.push_log(
                LogLevel::Warn,
                format!("请求被拒绝：{}（{reason}）。", event.file),
            );
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
                status: "传输中".into(),
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
    let throughput = if state.transfers.is_empty() {
        "—".to_string()
    } else if elapsed > 0.0 {
        format!("{}/s", format_bytes((delta as f32 / elapsed) as u64))
    } else {
        "—".to_string()
    };
    state.throughput_sample = (now, total_bytes);

    state.refresh_recent_model();
    app.set_transfers(ModelRc::from(state.transfers_model.clone()));
    if state.logs_dirty {
        state.rebuild_log_model();
        app.set_logs(ModelRc::from(state.log_model.clone()));
        state.logs_dirty = false;
    }
    app.set_log_total_count(i32::try_from(state.logs.len()).unwrap_or(i32::MAX));
    app.set_recent_dirs(ModelRc::from(state.recent_model.clone()));
    app.set_active_text(format!("{} 个进行中", state.transfers.len()).into());
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
        assert_eq!(root_status_text(&config.root_dir), "未指定根目录");
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
        assert_eq!(root_status_text(&restored.root_dir), "share");
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
            root_from_text("  \r\n"),
            Err("请先选择根目录。".to_string())
        );
        assert_eq!(
            root_from_text("  C:/srv/tftp  "),
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
        let mut options = build_address_choices(&[ip("127.0.0.1")], &[ip("10.0.0.7")]);
        assert!(is_selected(&options, "127.0.0.1"));

        toggle_address_choice(&mut options, ip("0.0.0.0"));
        assert!(is_selected(&options, "0.0.0.0"));
        assert!(!is_selected(&options, "127.0.0.1"));
        assert!(!is_selected(&options, "10.0.0.7"));
        assert_eq!(options.iter().filter(|option| option.selected).count(), 1);
    }

    #[test]
    fn choosing_a_specific_address_clears_all_networks() {
        let mut options = build_address_choices(&[ip("0.0.0.0")], &[ip("10.0.0.7")]);
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
        let mut options = build_address_choices(&[ip("0.0.0.0")], &[]);
        toggle_address_choice(&mut options, ip("0.0.0.0"));
        assert!(is_selected(&options, "0.0.0.0"));
        assert_eq!(options.iter().filter(|option| option.selected).count(), 1);
    }

    #[test]
    fn clearing_the_last_address_falls_back_to_all_networks() {
        let mut options = build_address_choices(&[ip("127.0.0.1")], &[ip("10.0.0.7")]);
        toggle_address_choice(&mut options, ip("127.0.0.1"));
        assert!(is_selected(&options, "0.0.0.0"));
        assert!(!is_selected(&options, "127.0.0.1"));
        assert_eq!(options.iter().filter(|option| option.selected).count(), 1);
    }

    #[test]
    fn toggling_an_unknown_address_is_ignored() {
        let mut options = build_address_choices(&[ip("0.0.0.0")], &[]);
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
        let mut state = UiState::new(
            config.clone(),
            RecentDirs::empty_for_tests(),
            LastRoot::empty_for_tests(),
            Arc::new(StdMutex::new(Some(Vec::new()))),
        );
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
        let options = build_address_choices(&[ip("127.0.0.1")], &[ip("192.168.1.5")]);
        let labels: Vec<&str> = options.iter().map(|option| option.label.as_str()).collect();
        assert_eq!(labels[0], "全部网络 (0.0.0.0)");
        assert_eq!(labels[1], "本地回环 (127.0.0.1)");
        assert!(labels.contains(&"192.168.1.5"));
        assert_eq!(options.iter().filter(|option| option.selected).count(), 1);
    }

    #[test]
    fn selected_addresses_missing_from_discovery_stay_visible() {
        let selected = vec![ip("::1"), ip("10.1.2.3")];
        let options = build_address_choices(&selected, &[]);
        assert_eq!(options.len(), 4);
        assert!(options.iter().any(|option| option.ip == ip("::1")));
        assert_eq!(options.iter().filter(|option| option.selected).count(), 2);
    }

    #[test]
    fn duplicate_selected_addresses_are_not_repeated() {
        let options = build_address_choices(&[ip("127.0.0.1"), ip("127.0.0.1")], &[]);
        assert_eq!(options.len(), 2);
        assert_eq!(options.iter().filter(|option| option.selected).count(), 1);
    }

    #[test]
    fn address_labels_cover_the_special_entries() {
        assert_eq!(address_label(ip("0.0.0.0")), "全部网络 (0.0.0.0)");
        assert_eq!(address_label(ip("127.0.0.1")), "本地回环 (127.0.0.1)");
        assert_eq!(address_label(ip("::")), "全部 IPv6 网络 (::)");
        assert_eq!(address_label(ip("::1")), "本地回环 IPv6 (::1)");
        assert_eq!(address_label(ip("192.168.1.5")), "192.168.1.5");
    }

    #[test]
    fn summary_is_short_for_many_addresses() {
        let options = build_address_choices(&[ip("0.0.0.0")], &[]);
        assert_eq!(bind_summary_text(&options), "全部网络 (0.0.0.0)");
        assert_eq!(bind_summary_text(&[]), "未选择地址");

        let many = build_address_choices(&[ip("0.0.0.0"), ip("10.0.0.1"), ip("10.0.0.2")], &[]);
        assert_eq!(
            bind_summary_text(&many),
            "全部网络 (0.0.0.0)、10.0.0.1 等 3 个地址"
        );
    }

    #[test]
    fn status_bar_summarises_many_listeners() {
        let one: Vec<SocketAddr> = vec!["127.0.0.1:69".parse().expect("addr")];
        assert_eq!(summarize_addrs(&one), "127.0.0.1:69");
        let four: Vec<SocketAddr> = (1..=4)
            .map(|last| format!("127.0.0.{last}:69").parse().expect("addr"))
            .collect();
        assert_eq!(summarize_addrs(&four), "127.0.0.1:69 等 4 个地址");
        assert_eq!(summarize_addrs(&[]), "—");
    }

    #[test]
    fn status_bar_root_shows_the_leaf_directory_name() {
        assert_eq!(root_display_name(Path::new("tftp-root")), "tftp-root");
        assert_eq!(root_display_name(Path::new("C:/srv/tftp")), "tftp");
        assert_eq!(root_status_text(Path::new("C:/srv/tftp")), "tftp");
        assert_eq!(root_status_text(Path::new("")), "未指定根目录");
    }

    #[test]
    fn remembering_the_root_keeps_the_last_selected_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = ServerConfig {
            root_dir: dir.path().to_path_buf(),
            ..ServerConfig::default()
        };
        let mut state = UiState::new(
            config,
            RecentDirs::empty_for_tests(),
            LastRoot::empty_for_tests(),
            Arc::new(StdMutex::new(Some(Vec::new()))),
        );
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

        let mut state = UiState::new(
            gui_default_config(),
            RecentDirs::empty_for_tests(),
            LastRoot::empty_for_tests(),
            Arc::new(StdMutex::new(Some(Vec::new()))),
        );
        let display = store_root(&mut state, share.clone());
        assert_eq!(state.config.root_dir, share);
        assert_eq!(display, display_path(&canonical));
        assert_eq!(state.recent.paths(), &[canonical.clone()][..]);
        assert_eq!(state.last_root.path(), Some(canonical.as_path()));
    }

    #[test]
    fn pending_dialogs_are_polled_with_their_target_and_snapshot() {
        let mut state = UiState::new(
            gui_default_config(),
            RecentDirs::empty_for_tests(),
            LastRoot::empty_for_tests(),
            Arc::new(StdMutex::new(Some(Vec::new()))),
        );
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
        let mut state = UiState::new(
            gui_default_config(),
            RecentDirs::empty_for_tests(),
            LastRoot::empty_for_tests(),
            Arc::new(StdMutex::new(Some(Vec::new()))),
        );

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
        let mut state = UiState::new(
            gui_default_config(),
            RecentDirs::empty_for_tests(),
            LastRoot::empty_for_tests(),
            Arc::new(StdMutex::new(Some(Vec::new()))),
        );
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
    fn product_icon_pixels_are_a_valid_rounded_tile() {
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
    }
}

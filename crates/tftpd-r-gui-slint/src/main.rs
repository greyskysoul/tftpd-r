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
mod netif;
mod recent;

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use slint::{Model, ModelRc, Timer, TimerMode, VecModel};
use tftpd_r_core::config::display_path;
use tftpd_r_core::events::{EventBus, EventKind, SessionId, TftpEvent};
use tftpd_r_core::server::ServerStatus;
use tftpd_r_core::ServerConfig;

use controller::EmbeddedController;
use recent::RecentDirs;

const MAX_LOG_ROWS: usize = 500;
const UI_TICK: Duration = Duration::from_millis(200);

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
    app_started: Instant,
    last_seq: u64,
    last_status_key: i32,
    logs: Rc<VecModel<LogRow>>,
    transfers_model: Rc<VecModel<TransferRow>>,
    transfers: HashMap<SessionId, TransferState>,
    recent: RecentDirs,
    recent_model: Rc<VecModel<RecentDir>>,
    config: ServerConfig,
    /// Root directory of the server instance that is starting or running.
    running_root: Option<PathBuf>,
    /// Message of the last failed start/restart, shown in the status bar.
    failed_message: Option<String>,
    throughput_sample: (Instant, u64),
    /// Bind addresses shown in the settings picker.
    address_options: Vec<AddressChoice>,
    /// Local interface addresses discovered on a background thread.
    discovered: Arc<StdMutex<Option<Vec<IpAddr>>>>,
    discovery_applied: bool,
    /// Pending native folder picker; polled from the UI timer.
    folder_picker: Option<std::sync::mpsc::Receiver<Option<PathBuf>>>,
}

impl UiState {
    fn new(
        config: ServerConfig,
        recent: RecentDirs,
        discovered: Arc<StdMutex<Option<Vec<IpAddr>>>>,
    ) -> Self {
        Self {
            app_started: Instant::now(),
            last_seq: 0,
            last_status_key: 0,
            logs: Rc::new(VecModel::default()),
            transfers_model: Rc::new(VecModel::default()),
            transfers: HashMap::new(),
            recent,
            recent_model: Rc::new(VecModel::default()),
            config,
            running_root: None,
            failed_message: None,
            throughput_sample: (Instant::now(), 0),
            address_options: Vec::new(),
            discovered,
            discovery_applied: false,
            folder_picker: None,
        }
    }

    fn push_log(&mut self, level: &str, message: impl Into<String>) {
        let message: String = message.into();
        let row = LogRow {
            time: format_clock(self.app_started.elapsed()).into(),
            level: level.into(),
            message: message.into(),
        };
        // Newest first: the newest event stays visible without scrolling.
        self.logs.insert(0, row);
        while self.logs.row_count() > MAX_LOG_ROWS {
            self.logs.remove(self.logs.row_count() - 1);
        }
    }

    fn refresh_recent_model(&mut self) {
        let active_root = self
            .running_root
            .clone()
            .unwrap_or_else(|| self.config.root_dir.clone());
        let active_root = std::fs::canonicalize(&active_root).unwrap_or(active_root);
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
    let (initial_config, config_error) = load_initial_config();
    let mut recent = RecentDirs::load();
    // Make sure the configured root shows up in the drop-down even on first run
    // or when it differs from the last used directory.
    recent.record(&initial_config.root_dir);

    // Interface discovery starts a platform helper; it must not block the
    // window creation, so it runs on a background thread and the options are
    // merged in from the UI timer once the result is available.
    let discovered = Arc::new(StdMutex::new(None));
    spawn_discovery(discovered.clone());

    let app = AppWindow::new()?;
    app.set_app_version(env!("CARGO_PKG_VERSION").into());

    let mut state = UiState::new(initial_config, recent, discovered);
    apply_config_to_settings(&app, &mut state);
    // Development aid for UI screenshots and manual layout checks: open a
    // dialog at startup. Not reachable without the environment variable.
    match std::env::var("TFTPD_R_GUI_PREVIEW").as_deref() {
        Ok("settings") => app.set_settings_open(true),
        Ok("picker") => {
            app.set_settings_open(true);
            app.set_settings_picker_open(true);
        }
        Ok("transfer") => {
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
    state.push_log("INFO", "TftpdR 面板已启动；默认只读 RRQ，上传已禁用。");
    if let Some(error) = config_error {
        state.push_log(
            "WARN",
            format!("config.toml 无效（{error}）；已使用安全默认值。"),
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
                state.push_log("ERROR", format!("重启失败：{error}"));
            }
            if state.apply_discovered() {
                apply_address_options(&app, &state);
            }
            if let Some(picked) = poll_folder_picker(&mut state) {
                handle_folder_pick(&app, &mut state, picked);
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

/// Load `config.toml` from the working directory, falling back to safe defaults.
/// The error is returned so the panel can show it in the activity log instead
/// of a console that does not exist for a windowed binary.
fn load_initial_config() -> (ServerConfig, Option<String>) {
    match std::fs::read_to_string("config.toml") {
        Ok(text) => match ServerConfig::from_toml_str(&text) {
            Ok(config) => (config, None),
            Err(error) => (ServerConfig::default(), Some(error.to_string())),
        },
        Err(_) => (ServerConfig::default(), None),
    }
}

fn poll_folder_picker(state: &mut UiState) -> Option<Option<PathBuf>> {
    let receiver = state.folder_picker.as_ref()?;
    let result = match receiver.try_recv() {
        Ok(result) => Some(result),
        Err(std::sync::mpsc::TryRecvError::Empty) => return None,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(None),
    };
    state.folder_picker = None;
    result
}

fn handle_folder_pick(app: &AppWindow, state: &mut UiState, picked: Option<PathBuf>) {
    match picked {
        Some(path) if path.is_dir() => {
            let display = display_root_path(&path);
            state.config.root_dir = path.clone();
            let _ = state.recent.record(&path);
            app.set_settings_root(display.clone().into());
            state.push_log("INFO", format!("根目录已设置为 {display}。"));
        }
        Some(path) => {
            state.push_log(
                "ERROR",
                format!("所选路径不是目录：{}", display_path(&path)),
            );
        }
        None => {}
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
                    state.push_log("ERROR", format!("设置无效：{message}"));
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
                    state.config = config;
                    state.failed_message = None;
                    state.running_root = Some(state.config.root_dir.clone());
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
                        "INFO",
                        format!("正在{verb}服务：监听 {bind}，根目录 {root}。"),
                    );
                    app.set_settings_error("".into());
                }
                Err(error) => {
                    let message = error.to_string();
                    state.failed_message = Some(message.clone());
                    state.push_log(
                        "ERROR",
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
                state.push_log("INFO", "正在停止服务；活动传输会按宽限期取消。");
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
                    state.config = config;
                    let root = state.config.root_dir.clone();
                    let _ = state.recent.record(&root);
                    state.push_log(
                        "INFO",
                        "设置已保存；服务运行中时，点击“重启”或用重启按钮使其生效。",
                    );
                    app.set_settings_error("".into());
                    app.set_settings_open(false);
                }
                Err(message) => {
                    app.set_settings_error(message.clone().into());
                    state.push_log("ERROR", format!("设置无效：{message}"));
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
            app.set_settings_open(false);
        });
    }
    // Toggle one bind address in the settings picker. The dialog stays open so
    // several addresses can be selected in one go.
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_address_toggled(move |address| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            let address = address.to_string();
            if let Some(option) = state
                .address_options
                .iter_mut()
                .find(|option| option.ip.to_string() == address)
            {
                option.selected = !option.selected;
                apply_address_options(&app, &state);
            }
        });
    }
    // Native folder picker for the root directory. The dialog runs in a helper
    // process on a background thread; the result arrives through the timer.
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_browse_root(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            if state.folder_picker.is_some() {
                state.push_log("INFO", "目录选择器已打开。");
                return;
            }
            let typed = PathBuf::from(app.get_settings_root().trim());
            let initial = if typed.is_dir() {
                Some(typed)
            } else {
                Some(state.config.root_dir.clone())
            };
            state.folder_picker = Some(folder_picker::pick_folder_async(initial));
            state.push_log("INFO", "已打开系统目录选择器…");
            refresh_models(&app, &mut state);
        });
    }
    // Clear the log view (the event ring is unaffected).
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_clear_logs(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let state = state.borrow();
            state.logs.set_vec(Vec::new());
            app.set_logs(ModelRc::from(state.logs.clone()));
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
                    "ERROR",
                    format!("无法切换根目录：{} 不存在或不是目录。", display_path(&path)),
                );
                refresh_models(&app, &mut state);
                return;
            }
            state.config.root_dir = path.clone();
            let _ = state.recent.record(&path);
            let root_text = display_root_path(&path);
            app.set_settings_root(root_text.clone().into());
            let running = matches!(
                controller.borrow_mut().poll_status(),
                ServerStatus::Starting | ServerStatus::Running { .. } | ServerStatus::Stopping
            );
            if running {
                state.push_log(
                    "INFO",
                    format!("根目录已切换为 {root_text}；点击“重启”后生效。"),
                );
            } else {
                state.push_log("INFO", format!("根目录已设置为 {root_text}。"));
            }
            refresh_models(&app, &mut state);
            let status = controller.borrow_mut().poll_status();
            apply_status(&app, &status, &state);
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
    let root_text = app.get_settings_root().trim().to_string();
    if root_text.is_empty() {
        return Err("根目录不能为空。".to_string());
    }
    let block_size: u16 = parse_setting(&app.get_settings_block_size(), "最大块大小")?;
    let timeout_secs: u64 = parse_setting(&app.get_settings_timeout(), "超时")?;
    let max_sessions: usize = parse_setting(&app.get_settings_max_sessions(), "最大并发会话")?;
    let config = ServerConfig {
        port,
        bind_addresses,
        root_dir: PathBuf::from(root_text),
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

fn parse_setting<T: std::str::FromStr>(text: &str, label: &str) -> Result<T, String> {
    let text = text.trim();
    text.parse::<T>()
        .map_err(|_| format!("{label}“{text}”不是有效的数字。"))
}

fn apply_config_to_settings(app: &AppWindow, state: &mut UiState) {
    let config = state.config.clone();
    app.set_settings_port(config.port.to_string().into());
    app.set_settings_root(config.display_root().into());
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
    // the settings dialog, the log and the recent-directory menu.
    let root = match status {
        ServerStatus::Starting | ServerStatus::Running { .. } | ServerStatus::Stopping => {
            let path = state
                .running_root
                .as_deref()
                .unwrap_or(&state.config.root_dir);
            root_display_name(path)
        }
        _ => root_display_name(&state.config.root_dir),
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

fn sync_running_root(state: &mut UiState, status: &ServerStatus) {
    if matches!(status, ServerStatus::Stopped | ServerStatus::Failed { .. }) {
        state.running_root = None;
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
                "INFO",
                format!(
                    "服务已就绪：监听 {}，根目录 {root}。",
                    summarize_addrs(local_addrs)
                ),
            );
        }
        ServerStatus::Stopped => {
            if state.last_status_key != 0 {
                state.push_log("INFO", "服务已停止。");
            }
        }
        ServerStatus::Failed { message } => {
            state.push_log("ERROR", format!("服务异常：{message}"));
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
                "INFO",
                format!("开始传输：{}（{size}），来自 {}", event.file, event.peer),
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
                "INFO",
                format!(
                    "传输完成：{file}（{}{duration}）。",
                    format_bytes(event.bytes)
                ),
            );
        }
        EventKind::Failed { reason } => {
            let transfer = state.transfers.remove(&event.session_id);
            let file = transfer.map_or_else(|| event.file.clone(), |transfer| transfer.file);
            state.push_log("WARN", format!("传输失败：{file}（{reason}）。"));
        }
        EventKind::Rejected { reason } => {
            state.push_log("WARN", format!("请求被拒绝：{}（{reason}）", event.file));
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
    app.set_logs(ModelRc::from(state.logs.clone()));
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

fn format_clock(elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        total / 3600,
        (total % 3600) / 60,
        total % 60
    )
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
    }
}

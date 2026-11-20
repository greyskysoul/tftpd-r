//! Slint desktop panel for the TFTP server (embedded mode).
//!
//! The GUI runs the `tftpd-r-core` server in its own Tokio runtime, polls the
//! bounded event bus from a Slint timer, and never blocks the UI thread while
//! stopping or restarting the server.

slint::include_modules!();

mod controller;
mod recent;

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
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
}

impl UiState {
    fn new(config: ServerConfig, recent: RecentDirs) -> Self {
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
    let initial_config = load_initial_config();
    let mut recent = RecentDirs::load();
    // Make sure the configured root shows up in the drop-down even on first run
    // or when it differs from the last used directory.
    recent.record(&initial_config.root_dir);

    let app = AppWindow::new()?;
    app.set_app_version(env!("CARGO_PKG_VERSION").into());
    apply_config_to_settings(&app, &initial_config);

    let mut state = UiState::new(initial_config, recent);
    state.refresh_recent_model();
    state.push_log("INFO", "TftpdR 面板已启动；默认只读 RRQ，上传已禁用。");
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

fn load_initial_config() -> ServerConfig {
    match std::fs::read_to_string("config.toml") {
        Ok(text) => match ServerConfig::from_toml_str(&text) {
            Ok(config) => config,
            Err(error) => {
                eprintln!("warning: config.toml is invalid ({error}); using safe defaults");
                ServerConfig::default()
            }
        },
        Err(_) => ServerConfig::default(),
    }
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
                    let bind = state.config.bind_address;
                    let root = display_root_path(&state.config.root_dir);
                    state.push_log(
                        "INFO",
                        format!("正在{verb}服务：绑定 {bind}，根目录 {root}。"),
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
            let state = state.borrow();
            apply_config_to_settings(&app, &state.config);
            app.set_settings_error("".into());
            app.set_settings_open(false);
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
    let bind_text = app.get_settings_bind().trim().to_string();
    let bind_address: SocketAddr = bind_text
        .parse()
        .map_err(|_| format!("绑定地址“{bind_text}”无效，应为 IP:端口。"))?;
    let root_text = app.get_settings_root().trim().to_string();
    if root_text.is_empty() {
        return Err("根目录不能为空。".to_string());
    }
    let block_size: u16 = parse_setting(&app.get_settings_block_size(), "最大块大小")?;
    let timeout_secs: u64 = parse_setting(&app.get_settings_timeout(), "超时")?;
    let max_sessions: usize = parse_setting(&app.get_settings_max_sessions(), "最大并发会话")?;
    let config = ServerConfig {
        bind_address,
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

fn apply_config_to_settings(app: &AppWindow, config: &ServerConfig) {
    app.set_settings_bind(config.bind_address.to_string().into());
    app.set_settings_root(config.display_root().into());
    app.set_settings_block_size(config.block_size.to_string().into());
    app.set_settings_timeout(config.timeout_secs.to_string().into());
    app.set_settings_max_sessions(config.max_sessions.to_string().into());
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
        ServerStatus::Running { local_addr } => local_addr.to_string(),
        _ => state.config.bind_address.to_string(),
    };
    let root = match status {
        ServerStatus::Starting | ServerStatus::Running { .. } | ServerStatus::Stopping => {
            let path = state
                .running_root
                .as_deref()
                .unwrap_or(&state.config.root_dir);
            display_root_path(path)
        }
        _ => display_root_path(&state.config.root_dir),
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
        ServerStatus::Running { local_addr } => {
            let root = state
                .running_root
                .as_deref()
                .unwrap_or(&state.config.root_dir);
            let root = display_root_path(root);
            state.push_log(
                "INFO",
                format!("服务已就绪：绑定 {local_addr}，根目录 {root}。"),
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
            if let Some(transfer) = state.transfers.remove(&event.session_id) {
                state.push_log(
                    "INFO",
                    format!(
                        "传输完成：{}（{}），耗时 {:.1} 秒。",
                        transfer.file,
                        format_bytes(event.bytes),
                        transfer.started.elapsed().as_secs_f32()
                    ),
                );
            }
        }
        EventKind::Failed { reason } => {
            if let Some(transfer) = state.transfers.remove(&event.session_id) {
                state.push_log("WARN", format!("传输失败：{}（{reason}）", transfer.file));
            } else {
                state.push_log("WARN", format!("传输失败（{reason}）"));
            }
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

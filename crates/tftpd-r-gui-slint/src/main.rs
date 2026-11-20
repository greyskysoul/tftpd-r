//! Slint desktop panel for the TFTP server (embedded mode).
//!
//! The GUI runs the `tftpd-r-core` server in its own Tokio runtime, polls the
//! bounded event bus from a Slint timer, and never blocks the UI thread while
//! stopping the server.

slint::include_modules!();

mod controller;

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use slint::{Model, ModelRc, Timer, TimerMode, VecModel};
use tftpd_r_core::events::{EventBus, EventKind, SessionId, TftpEvent};
use tftpd_r_core::server::ServerStatus;
use tftpd_r_core::ServerConfig;

use controller::EmbeddedController;

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
    logs: Rc<VecModel<LogRow>>,
    transfers_model: Rc<VecModel<TransferRow>>,
    transfers: HashMap<SessionId, TransferState>,
    config: ServerConfig,
    throughput_sample: (Instant, u64),
}

impl UiState {
    fn new(config: ServerConfig) -> Self {
        Self {
            app_started: Instant::now(),
            last_seq: 0,
            logs: Rc::new(VecModel::default()),
            transfers_model: Rc::new(VecModel::default()),
            transfers: HashMap::new(),
            config,
            throughput_sample: (Instant::now(), 0),
        }
    }

    fn push_log(&mut self, level: &str, message: impl Into<String>) {
        let message: String = message.into();
        let row = LogRow {
            time: format!("{:6.1}s", self.app_started.elapsed().as_secs_f32()).into(),
            level: level.into(),
            message: message.into(),
        };
        self.logs.push(row);
        while self.logs.row_count() > MAX_LOG_ROWS {
            self.logs.remove(0);
        }
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
    let app = AppWindow::new()?;
    apply_config_to_settings(&app, &initial_config);
    let mut state = UiState::new(initial_config);
    state.push_log(
        "INFO",
        "Application started; uploads disabled, read-only RRQ only.",
    );
    let state = Rc::new(RefCell::new(state));
    let controller = Rc::new(RefCell::new(EmbeddedController::new()?));

    apply_status(&app, &ServerStatus::Stopped, &state.borrow().config);
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
            let mut state = state.borrow_mut();
            drain_events(&events, &mut state);
            refresh_models(&app, &mut state);
            apply_status(&app, &status, &state.config);
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
    {
        let weak = app.as_weak();
        let controller = controller.clone();
        let state = state.clone();
        app.on_start_requested(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            let config = match read_config(&app, &state.config) {
                Ok(config) => config,
                Err(message) => {
                    app.set_settings_error(message.clone().into());
                    state.push_log("ERROR", format!("Invalid settings: {message}"));
                    refresh_models(&app, &mut state);
                    return;
                }
            };
            match controller.borrow_mut().start(config.clone()) {
                Ok(()) => {
                    state.config = config;
                    app.set_settings_error("".into());
                    app.set_error_text("".into());
                    let bind = state.config.bind_address;
                    state.push_log("INFO", format!("Starting server on {bind}..."));
                }
                Err(error) => {
                    let message = format!("Could not start server: {error}");
                    app.set_settings_error(message.clone().into());
                    state.push_log("ERROR", message);
                }
            }
            refresh_models(&app, &mut state);
        });
    }
    {
        let weak = app.as_weak();
        let controller = controller.clone();
        let state = state.clone();
        app.on_stop_requested(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            if controller.borrow().is_running() && controller.borrow_mut().stop() {
                state.push_log(
                    "INFO",
                    "Stopping server; active transfers are cancelled gracefully.",
                );
            }
            refresh_models(&app, &mut state);
            let status = controller.borrow_mut().poll_status();
            apply_status(&app, &status, &state.config);
        });
    }
    {
        let weak = app.as_weak();
        let state = state.clone();
        app.on_apply_settings(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            match read_config(&app, &state.config) {
                Ok(config) => {
                    state.push_log(
                        "INFO",
                        "Settings validated and stored; they take effect the next time the server starts.",
                    );
                    state.config = config.clone();
                    apply_config_to_settings(&app, &config);
                    app.set_settings_error("".into());
                    app.set_error_text("".into());
                }
                Err(message) => {
                    app.set_settings_error(message.clone().into());
                    state.push_log("ERROR", format!("Invalid settings: {message}"));
                }
            }
            refresh_models(&app, &mut state);
        });
    }
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
}

fn read_config(app: &AppWindow, fallback: &ServerConfig) -> Result<ServerConfig, String> {
    let bind_text = app.get_settings_bind().trim().to_string();
    let bind_address: SocketAddr = bind_text
        .parse()
        .map_err(|error| format!("Bind address '{bind_text}' is invalid: {error}"))?;
    let root_text = app.get_settings_root().trim().to_string();
    if root_text.is_empty() {
        return Err("Root directory must not be empty.".to_string());
    }
    let block_size: u16 = parse_setting(&app.get_settings_block_size(), "Max block size")?;
    let timeout_secs: u64 = parse_setting(&app.get_settings_timeout(), "Timeout")?;
    let max_sessions: usize = parse_setting(&app.get_settings_max_sessions(), "Max sessions")?;
    let config = ServerConfig {
        bind_address,
        root_dir: PathBuf::from(root_text),
        block_size,
        timeout_secs,
        max_sessions,
        ..fallback.clone()
    };
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

fn parse_setting<T: std::str::FromStr>(text: &str, label: &str) -> Result<T, String> {
    let text = text.trim();
    text.parse::<T>()
        .map_err(|_| format!("{label} '{text}' is not a valid number."))
}

fn apply_config_to_settings(app: &AppWindow, config: &ServerConfig) {
    app.set_settings_bind(config.bind_address.to_string().into());
    app.set_settings_root(config.display_root().into());
    app.set_settings_block_size(config.block_size.to_string().into());
    app.set_settings_timeout(config.timeout_secs.to_string().into());
    app.set_settings_max_sessions(config.max_sessions.to_string().into());
    app.set_bind_text(config.bind_address.to_string().into());
    app.set_root_text(config.display_root().into());
}

fn apply_status(app: &AppWindow, status: &ServerStatus, config: &ServerConfig) {
    let (text, kind, can_start, can_stop, bind) = match status {
        ServerStatus::Starting => (
            "Starting".to_string(),
            2,
            false,
            true,
            config.bind_address.to_string(),
        ),
        ServerStatus::Running { local_addr } => (
            "Running".to_string(),
            1,
            false,
            true,
            local_addr.to_string(),
        ),
        ServerStatus::Stopping => (
            "Stopping".to_string(),
            3,
            false,
            false,
            config.bind_address.to_string(),
        ),
        ServerStatus::Stopped => (
            "Stopped".to_string(),
            0,
            true,
            false,
            config.bind_address.to_string(),
        ),
        ServerStatus::Failed { message } => (
            format!("Failed: {message}"),
            4,
            true,
            false,
            config.bind_address.to_string(),
        ),
    };
    app.set_status_text(text.into());
    app.set_status_kind(kind);
    app.set_can_start(can_start);
    app.set_can_stop(can_stop);
    app.set_bind_text(bind.into());
    app.set_root_text(config.display_root().into());
    let session_count = app.get_transfers().row_count();
    app.set_session_count(session_count.to_string().into());
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
                .map_or_else(|| "unknown size".to_string(), format_bytes);
            state.push_log(
                "INFO",
                format!(
                    "Transfer started: {} ({size}) from {}",
                    event.file, event.peer
                ),
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
                        "Transfer completed: {} ({}) in {:.1}s",
                        transfer.file,
                        format_bytes(event.bytes),
                        transfer.started.elapsed().as_secs_f32()
                    ),
                );
            }
        }
        EventKind::Failed { reason } => {
            if let Some(transfer) = state.transfers.remove(&event.session_id) {
                state.push_log(
                    "WARN",
                    format!("Transfer failed: {} ({reason})", transfer.file),
                );
            } else {
                state.push_log("WARN", format!("Transfer failed ({reason})"));
            }
        }
        EventKind::Rejected { reason } => {
            state.push_log(
                "WARN",
                format!("Request rejected: {} ({reason})", event.file),
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
                progress,
                status: "transferring".into(),
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

    app.set_transfers(ModelRc::from(state.transfers_model.clone()));
    app.set_logs(ModelRc::from(state.logs.clone()));
    app.set_session_count(state.transfers.len().to_string().into());
    app.set_throughput_text(throughput.into());
    let error_text = app.get_settings_error();
    app.set_error_text(error_text);
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

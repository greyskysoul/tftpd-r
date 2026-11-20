//! Logging initialisation and the tracing-backed event sink.

use std::io::IsTerminal;

use tftpd_r_core::events::{EventKind, EventSink, TftpEvent};
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

/// Forward transfer lifecycle events to `tracing`.
///
/// Progress events are logged at DEBUG so a large transfer does not flood the
/// console at the default INFO level.
pub struct TracingSink;

impl EventSink for TracingSink {
    fn emit(&self, event: TftpEvent) {
        match &event.kind {
            EventKind::Started => info!(
                session = event.session_id,
                peer = %event.peer,
                file = %event.file,
                size = event.total.unwrap_or(0),
                "transfer started"
            ),
            EventKind::Progress => debug!(
                session = event.session_id,
                peer = %event.peer,
                file = %event.file,
                bytes = event.bytes,
                "transfer progress"
            ),
            EventKind::Completed => info!(
                session = event.session_id,
                peer = %event.peer,
                file = %event.file,
                bytes = event.bytes,
                "transfer completed"
            ),
            EventKind::Failed { reason } => warn!(
                session = event.session_id,
                peer = %event.peer,
                file = %event.file,
                reason = %reason,
                "transfer failed"
            ),
            EventKind::Rejected { reason } => warn!(
                session = event.session_id,
                peer = %event.peer,
                file = %event.file,
                reason = %reason,
                "request rejected"
            ),
        }
    }
}

/// Initialise the global tracing subscriber.
///
/// Precedence: `--log <filter>` > `TFTPD_R_LOG` environment variable > `info`.
/// Errors from a second initialisation (for example under a test harness) are
/// ignored.
pub fn init(cli_filter: Option<&str>) {
    let filter = match cli_filter {
        Some(filter) => EnvFilter::new(filter),
        None => EnvFilter::try_from_env("TFTPD_R_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(std::io::stdout().is_terminal())
        .try_init();
}

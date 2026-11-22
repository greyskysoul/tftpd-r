//! Logging initialisation and the tracing-backed event sink.

use std::io::IsTerminal;
use std::str::FromStr;

use tftpd_r_core::events::{EventKind, EventSink, TftpEvent};
use tracing::{debug, info, warn};
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Layer;

/// Maximum level used when the filter specification does not select one.
const DEFAULT_LEVEL: LevelFilter = LevelFilter::INFO;

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

/// Parse a `tracing` filter specification into a per-target filter.
///
/// Only the level-based subset of the `tracing` environment-filter syntax is
/// supported; span- and field-scoped directives are not:
///
/// - `level` — maximum level for every target (`off`, `error`, `warn`, `info`,
///   `debug`, `trace`, or a number from 0 (`off`) to 5 (`trace`)),
/// - `target=level` — maximum level for every target whose name starts with
///   `target`,
/// - `target` — shorthand for `target=trace`.
///
/// Directives are separated by commas and surrounding spaces are ignored.
/// Directives that cannot be parsed are reported on stderr and skipped, so a
/// typo never prevents the daemon from starting.
///
/// The default level is [`DEFAULT_LEVEL`] unless a bare level sets another one.
/// Unlike `tracing`'s `EnvFilter`, a target-only specification therefore
/// narrows that target instead of turning every other target off:
/// `TFTPD_R_LOG=tftpd_r_core=debug` still logs the daemon's own INFO lines.
fn parse_filter(spec: &str) -> Targets {
    let mut targets = Targets::new();
    let mut default = None;
    for directive in spec.split(',') {
        let directive = directive.trim();
        if directive.is_empty() {
            continue;
        }
        if let Some((target, level)) = directive.split_once('=') {
            let (target, level) = (target.trim(), level.trim());
            let parsed = if target.is_empty() || level.is_empty() {
                None
            } else {
                LevelFilter::from_str(level).ok()
            };
            match parsed {
                Some(level) => targets = targets.with_target(target, level),
                None => warn_invalid(directive),
            }
        } else {
            match LevelFilter::from_str(directive) {
                Ok(level) => default = Some(level),
                // A bare word names a target, as in the tracing syntax.
                Err(_) => targets = targets.with_target(directive, LevelFilter::TRACE),
            }
        }
    }
    targets.with_default(default.unwrap_or(DEFAULT_LEVEL))
}

/// Report a directive that cannot be parsed without failing startup.
fn warn_invalid(directive: &str) {
    eprintln!("warning: ignoring invalid log filter directive {directive:?}");
}

/// Initialise the global tracing subscriber.
///
/// Precedence: `--log <filter>` > `TFTPD_R_LOG` environment variable > `info`.
/// Errors from a second initialisation (for example under a test harness) are
/// ignored.
pub fn init(cli_filter: Option<&str>) {
    let spec = match cli_filter {
        Some(filter) => filter.to_string(),
        None => std::env::var("TFTPD_R_LOG").unwrap_or_default(),
    };
    let filter = parse_filter(&spec);
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_target(false)
            .with_ansi(std::io::stdout().is_terminal())
            .with_writer(std::io::stdout)
            .with_filter(filter),
    );
    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::Level;

    #[test]
    fn empty_spec_uses_the_default_level() {
        let filter = parse_filter("");
        assert!(filter.would_enable("tftpd_r_core", &Level::INFO));
        assert!(filter.would_enable("tftpd_r", &Level::INFO));
        assert!(!filter.would_enable("tftpd_r_core", &Level::DEBUG));
    }

    #[test]
    fn bare_level_sets_the_default() {
        let filter = parse_filter("debug");
        assert!(filter.would_enable("anything", &Level::DEBUG));
        assert!(!filter.would_enable("anything", &Level::TRACE));
    }

    #[test]
    fn target_directive_narrows_only_that_target() {
        let filter = parse_filter("tftpd_r_core=debug");
        assert!(filter.would_enable("tftpd_r_core::session", &Level::DEBUG));
        assert!(!filter.would_enable("tftpd_r_core::session", &Level::TRACE));
        assert!(filter.would_enable("tftpd_r", &Level::INFO));
        assert!(!filter.would_enable("tftpd_r", &Level::DEBUG));
    }

    #[test]
    fn bare_target_means_trace() {
        let filter = parse_filter("tftpd_r_core, info");
        assert!(filter.would_enable("tftpd_r_core", &Level::TRACE));
        assert!(filter.would_enable("other", &Level::INFO));
        assert!(!filter.would_enable("other", &Level::DEBUG));
    }

    #[test]
    fn numeric_and_uppercase_levels_are_accepted() {
        let info = parse_filter("3");
        assert!(info.would_enable("x", &Level::INFO));
        assert!(!info.would_enable("x", &Level::DEBUG));
        assert!(parse_filter("DEBUG").would_enable("x", &Level::DEBUG));
    }

    #[test]
    fn invalid_directives_are_skipped() {
        let filter = parse_filter("=info, tftpd_r_core=loud, info");
        assert!(filter.would_enable("tftpd_r_core", &Level::INFO));
        assert!(!filter.would_enable("tftpd_r_core", &Level::DEBUG));
    }
}

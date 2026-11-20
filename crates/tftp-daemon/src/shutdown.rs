//! Shutdown signal handling.

/// Wait for the platform shutdown signal.
///
/// * Windows: Ctrl-C or Ctrl-Break (both console control events).
/// * Unix: SIGINT or SIGTERM.
#[cfg(windows)]
pub async fn wait_for_shutdown_signal() -> &'static str {
    use tokio::signal::windows::{ctrl_break, ctrl_c};

    let mut interrupt = match ctrl_c() {
        Ok(stream) => stream,
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
            return "Ctrl-C";
        }
    };
    let mut brk = match ctrl_break() {
        Ok(stream) => stream,
        Err(_) => {
            let _ = interrupt.recv().await;
            return "Ctrl-C";
        }
    };
    tokio::select! {
        _ = interrupt.recv() => "Ctrl-C",
        _ = brk.recv() => "Ctrl-Break",
    }
}

/// Wait for the platform shutdown signal.
///
/// * Windows: Ctrl-C or Ctrl-Break (both console control events).
/// * Unix: SIGINT or SIGTERM.
#[cfg(unix)]
pub async fn wait_for_shutdown_signal() -> &'static str {
    use tokio::signal::unix::{signal, SignalKind};

    let mut terminate = match signal(SignalKind::terminate()) {
        Ok(stream) => stream,
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
            return "SIGINT";
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => "SIGINT",
        _ = terminate.recv() => "SIGTERM",
    }
}

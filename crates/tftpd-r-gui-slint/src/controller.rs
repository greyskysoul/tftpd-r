//! Embedded-mode controller used by the Slint GUI.
//!
//! The GUI owns the Tokio runtime and the `tftpd-r-core` server handle; there is no
//! daemon or IPC dependency in this mode (see `docs/adr/0004-embedded-only.md`).
//! All public methods are meant to be called from the UI thread.

use std::sync::Arc;
use std::time::Duration;

use tftpd_r_core::events::{EventBus, EventSink};
use tftpd_r_core::server::{ServerHandle, ServerStatus};
use tftpd_r_core::{ServerConfig, ServerError, TftpServer};

/// Errors returned when starting the embedded server.
#[derive(Debug, thiserror::Error)]
pub enum ControllerError {
    /// The Tokio runtime could not be created.
    #[error("could not create the Tokio runtime: {0}")]
    Runtime(std::io::Error),
    /// A server is already running in this controller.
    #[error("the server is already running")]
    AlreadyRunning,
    /// The server failed to start (configuration, sandbox root or bind).
    #[error(transparent)]
    Start(#[from] ServerError),
}

/// Owns the Tokio runtime, event bus and optional running server.
pub struct EmbeddedController {
    runtime: tokio::runtime::Runtime,
    events: Arc<EventBus>,
    server: Option<ServerHandle>,
    stop_task: Option<tokio::task::JoinHandle<()>>,
    status: ServerStatus,
    status_rx: Option<tokio::sync::watch::Receiver<ServerStatus>>,
    pending_restart: Option<ServerConfig>,
    last_error: Option<String>,
}

impl EmbeddedController {
    /// Create the controller and its Tokio runtime.
    ///
    /// # Errors
    ///
    /// Returns [`ControllerError::Runtime`] when the runtime cannot be built.
    pub fn new() -> Result<Self, ControllerError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("tftp-gui")
            .enable_all()
            .build()
            .map_err(ControllerError::Runtime)?;
        Ok(Self {
            runtime,
            events: EventBus::default_shared(),
            server: None,
            stop_task: None,
            status: ServerStatus::Stopped,
            status_rx: None,
            pending_restart: None,
            last_error: None,
        })
    }

    /// Event bus that the server publishes to.
    #[must_use]
    pub fn events(&self) -> Arc<EventBus> {
        self.events.clone()
    }

    /// Start the embedded server. Fails if one is already running.
    ///
    /// # Errors
    ///
    /// Returns [`ControllerError::AlreadyRunning`] or [`ControllerError::Start`].
    pub fn start(&mut self, config: ServerConfig) -> Result<(), ControllerError> {
        if self.server.is_some() {
            return Err(ControllerError::AlreadyRunning);
        }
        self.status = ServerStatus::Starting;
        let events: Arc<dyn EventSink> = self.events.clone();
        let handle = match self.runtime.block_on(TftpServer::start(config, events)) {
            Ok(handle) => handle,
            Err(error) => {
                self.status = ServerStatus::Stopped;
                return Err(error.into());
            }
        };
        self.status_rx = Some(handle.status_receiver());
        self.status = ServerStatus::Running {
            local_addrs: handle.local_addrs().to_vec(),
        };
        self.server = Some(handle);
        Ok(())
    }

    /// Start the server, or stop and restart it when one is running.
    ///
    /// A restart is asynchronous: when a server is running (or still shutting
    /// down) the new configuration is queued and applied from [`Self::poll_status`]
    /// once the old server reached `Stopped`. Restart failures are reported by
    /// [`Self::take_error`] on the polling cycle.
    ///
    /// # Errors
    ///
    /// Returns [`ControllerError::Start`] when no server is running and the new
    /// configuration cannot be started immediately.
    pub fn restart(&mut self, config: ServerConfig) -> Result<(), ControllerError> {
        if self.server.is_none() && !matches!(self.status, ServerStatus::Stopping) {
            return self.start(config);
        }
        self.pending_restart = Some(config);
        if self.server.is_some() {
            self.stop();
        }
        Ok(())
    }

    /// Request shutdown without blocking the UI thread. Returns `false` when no
    /// server is running.
    pub fn stop(&mut self) -> bool {
        let Some(handle) = self.server.take() else {
            return false;
        };
        self.status_rx = Some(handle.status_receiver());
        self.status = ServerStatus::Stopping;
        self.stop_task = Some(self.runtime.spawn(async move {
            handle.stop().await;
        }));
        true
    }

    /// Refresh and return the last known status.
    ///
    /// When a restart was requested, the queued configuration is started here
    /// as soon as the previous server has stopped.
    pub fn poll_status(&mut self) -> ServerStatus {
        if let Some(receiver) = self.status_rx.as_mut() {
            // `borrow_and_update` works even after the watch sender is dropped
            // (which happens as soon as the server task has finished), so the
            // final `Stopped` state is never lost.
            self.status = receiver.borrow_and_update().clone();
        }
        // `stop()` (and a queued restart) takes the handle immediately, but the
        // watch channel can still hold `Running` for one tick. Without a handle
        // the server is at most stopping, never running.
        if self.server.is_none()
            && matches!(
                self.status,
                ServerStatus::Starting | ServerStatus::Running { .. }
            )
        {
            self.status = ServerStatus::Stopping;
        }
        // A queued restart means the old server is on its way out. The watch
        // channel can still hold its last `Running` value for one tick; never
        // report the controller as running while a restart is pending.
        if self.pending_restart.is_some()
            && matches!(
                self.status,
                ServerStatus::Starting | ServerStatus::Running { .. }
            )
        {
            self.status = ServerStatus::Stopping;
        }
        self.complete_pending_restart();
        self.status.clone()
    }

    /// Take the error of the most recent failed queued restart, if any.
    pub fn take_error(&mut self) -> Option<String> {
        self.last_error.take()
    }

    fn complete_pending_restart(&mut self) {
        if !matches!(
            self.status,
            ServerStatus::Stopped | ServerStatus::Failed { .. }
        ) {
            return;
        }
        let Some(config) = self.pending_restart.take() else {
            return;
        };
        if let Err(error) = self.start(config) {
            self.status = ServerStatus::Stopped;
            self.last_error = Some(error.to_string());
        }
    }

    /// Whether the server is starting or running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(
            self.status,
            ServerStatus::Starting | ServerStatus::Running { .. }
        )
    }

    /// Stop the server (if any) and wait for pending shutdown work, bounded by
    /// the server's configured grace period. Call once when the window closes.
    pub fn shutdown(&mut self) {
        self.pending_restart = None;
        if let Some(handle) = self.server.take() {
            self.runtime.block_on(handle.stop());
        }
        if let Some(task) = self.stop_task.take() {
            let _ = self
                .runtime
                .block_on(async { tokio::time::timeout(Duration::from_secs(10), task).await });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::time::Instant;

    fn config(root: &Path) -> ServerConfig {
        ServerConfig {
            root_dir: root.to_path_buf(),
            port: 0,
            bind_addresses: vec!["127.0.0.1".parse().expect("addr")],
            timeout_secs: 1,
            max_retries: 1,
            shutdown_grace_secs: 2,
            ..ServerConfig::default()
        }
    }

    #[test]
    fn start_and_stop_embedded_server() {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join("hello.txt"), b"hello").expect("write");
        let mut controller = EmbeddedController::new().expect("controller");
        controller.start(config(dir.path())).expect("start");
        assert!(controller.is_running());
        let status = controller.poll_status();
        let address = status
            .local_addrs()
            .first()
            .copied()
            .expect("running address");
        assert!(address.ip().is_loopback());

        assert!(matches!(
            controller.start(config(dir.path())),
            Err(ControllerError::AlreadyRunning)
        ));

        assert!(controller.stop());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = controller.poll_status();
            if matches!(status, ServerStatus::Stopped) {
                break;
            }
            assert!(Instant::now() < deadline, "server did not stop: {status:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!controller.stop());
        controller.shutdown();
    }

    #[test]
    fn status_never_reports_running_after_a_stop_was_requested() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut controller = EmbeddedController::new().expect("controller");
        controller.start(config(dir.path())).expect("start");
        assert!(controller.is_running());

        assert!(controller.stop());
        let status = controller.poll_status();
        assert!(
            !matches!(
                status,
                ServerStatus::Starting | ServerStatus::Running { .. }
            ),
            "status after stop: {status:?}"
        );
        controller.shutdown();
    }

    #[test]
    fn invalid_configuration_does_not_start_the_server() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut controller = EmbeddedController::new().expect("controller");
        let mut invalid = config(dir.path());
        invalid.root_dir = dir.path().join("missing");
        assert!(controller.start(invalid).is_err());
        assert!(!controller.is_running());
        controller.shutdown();
    }

    #[test]
    fn restart_waits_for_the_old_server_and_starts_the_new_one() {
        let first = tempfile::tempdir().expect("first dir");
        let second = tempfile::tempdir().expect("second dir");
        fs::write(first.path().join("a.txt"), b"a").expect("write a");
        fs::write(second.path().join("b.txt"), b"b").expect("write b");

        let mut controller = EmbeddedController::new().expect("controller");
        controller.start(config(first.path())).expect("start");
        assert!(controller.is_running());

        controller
            .restart(config(second.path()))
            .expect("queue restart");
        // The old server is shutting down; the controller must report progress.
        assert!(!controller.is_running());

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = controller.poll_status();
            if matches!(status, ServerStatus::Running { .. }) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "restart did not complete: {status:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(controller.is_running());

        assert!(controller.stop());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = controller.poll_status();
            if matches!(status, ServerStatus::Stopped) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "server did not stop after restart: {status:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        controller.shutdown();
    }

    #[test]
    fn failed_restart_is_reported_without_leaving_the_server_running() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut controller = EmbeddedController::new().expect("controller");
        controller.start(config(dir.path())).expect("start");

        let mut invalid = config(dir.path());
        invalid.root_dir = dir.path().join("missing");
        controller.restart(invalid).expect("queue restart");

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut error = None;
        while error.is_none() {
            controller.poll_status();
            error = controller.take_error();
            assert!(
                Instant::now() < deadline,
                "restart failure was not reported"
            );
            if error.is_none() {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        assert!(!controller.is_running());
        assert!(error.expect("error").contains("root"));
        controller.shutdown();
    }
}

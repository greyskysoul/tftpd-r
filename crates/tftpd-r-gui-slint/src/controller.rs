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
            local_addr: handle.local_addr(),
        };
        self.server = Some(handle);
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
    pub fn poll_status(&mut self) -> ServerStatus {
        if let Some(receiver) = self.status_rx.as_mut() {
            // `borrow_and_update` works even after the watch sender is dropped
            // (which happens as soon as the server task has finished), so the
            // final `Stopped` state is never lost.
            self.status = receiver.borrow_and_update().clone();
        }
        self.status.clone()
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
            bind_address: "127.0.0.1:0".parse().expect("addr"),
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
        let address = status.local_addr().expect("running address");
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
    fn invalid_configuration_does_not_start_the_server() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut controller = EmbeddedController::new().expect("controller");
        let mut invalid = config(dir.path());
        invalid.root_dir = dir.path().join("missing");
        assert!(controller.start(invalid).is_err());
        assert!(!controller.is_running());
        controller.shutdown();
    }
}

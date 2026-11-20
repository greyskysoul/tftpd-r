//! TFTP protocol engine, sandboxed file access, configuration and server runtime.
//!
//! `tftpd-r-core` has no dependency on the GUI, the daemon binary or any IPC layer.
//! It can be embedded in a GUI front-end (see `tftpd-r-gui-slint`) or driven by the
//! headless daemon (`tftpd-r-daemon`).
//!
//! # Scope of this release (MVP)
//!
//! * RFC 1350 read transfers (RRQ) over UDP, with a dedicated transfer ID (TID)
//!   socket per session.
//! * RFC 2347/2348/2349 option negotiation: `blksize`, `timeout`, `tsize`.
//! * `octet` mode only. `netascii`/`mail` requests are rejected explicitly.
//! * WRQ uploads are parsed but always rejected: upload support has not been
//!   security-reviewed yet. [`ServerConfig::validate`] refuses a configuration
//!   with `allow_upload = true` so an operator cannot believe uploads are on.
//! * A sandboxed root directory: path traversal, device names, alternate data
//!   streams and reparse points (symlinks/junctions) are rejected.
//!
//! # Runtime ownership
//!
//! The crate uses Tokio primitives but does not create a runtime. The caller
//! owns the runtime and must keep it alive for as long as [`ServerHandle`]
//! exists. See `docs/adr/0001-runtime-ownership.md`.
//!
//! # Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use tftpd_r_core::{EventBus, ServerConfig, TftpServer};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let root = std::env::temp_dir().join("tftp-root");
//! std::fs::create_dir_all(&root)?;
//! let mut config = ServerConfig::default();
//! config.root_dir = root;
//! config.validate()?;
//!
//! let events = EventBus::default_shared();
//! let server = TftpServer::start(config, events).await?;
//! println!("listening on {:?}", server.local_addrs());
//! server.stop().await;
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

pub mod config;
pub mod error;
pub mod events;
pub mod filesystem;
pub mod protocol;
pub mod server;
pub mod session;

pub use config::{ConfigError, ServerConfig};
pub use error::{is_privileged_port, DecodeError, EncodeError, ErrorCode, ServerError};
pub use events::{EventBus, EventKind, EventSink, SessionId, TftpEvent};
pub use filesystem::{FileError, OpenedFile, Sandbox, SandboxError};
pub use protocol::{Mode, Packet, Request};
pub use server::{ServerHandle, ServerStatus, TftpServer};

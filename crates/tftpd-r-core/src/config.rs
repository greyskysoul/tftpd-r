//! Server configuration with validation and TOML (de)serialisation.
//!
//! Defaults are deliberately conservative: loopback binding, read-only access,
//! no uploads, modest concurrency and 64 MiB per-file limit.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Default UDP port: the standard TFTP port.
///
/// Port 69 is privileged on Windows (administrator token) and Unix (root or
/// `CAP_NET_BIND_SERVICE`). A start without those privileges fails with an
/// actionable hint; the server never falls back to another port on its own.
pub const DEFAULT_PORT: u16 = 69;

/// Default listen address: loopback only.
pub const DEFAULT_BIND_ADDRESSES: [IpAddr; 1] = [IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)];

/// Upper bound for the number of configured listen addresses.
///
/// Every address owns a listening socket and its own accept loop, so the list
/// is bounded like every other resource in this server.
pub const MAX_BIND_ADDRESSES: usize = 16;

/// Minimum allowed `block_size` (RFC 2348).
pub const MIN_CONFIG_BLOCK_SIZE: u16 = 8;
/// Maximum allowed `block_size` (RFC 2348).
pub const MAX_CONFIG_BLOCK_SIZE: u16 = 65_464;
/// Default maximum block size offered to clients (baseline RFC 1350 value).
pub const DEFAULT_BLOCK_SIZE: u16 = 512;
/// Default per-transfer timeout.
pub const DEFAULT_TIMEOUT_SECS: u64 = 3;
/// Default retransmission count before a transfer is failed.
pub const DEFAULT_MAX_RETRIES: u32 = 5;
/// Default concurrent session limit.
pub const DEFAULT_MAX_SESSIONS: usize = 16;
/// Default per-file size limit (64 MiB).
pub const DEFAULT_MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Default maximum total duration of one transfer (1 hour).
pub const DEFAULT_MAX_TRANSFER_DURATION_SECS: u64 = 3600;
/// Default grace period for draining sessions during shutdown.
pub const DEFAULT_SHUTDOWN_GRACE_SECS: u64 = 5;
/// Default event ring capacity.
pub const DEFAULT_EVENT_BUFFER: usize = 1024;

/// Complete server configuration.
///
/// The struct is serde-friendly so both the daemon (TOML file) and the GUI
/// (form state) can use the same validation rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    /// UDP port shared by every listen address. Port `0` is accepted for tests
    /// and temporary instances: the first listener picks an ephemeral port,
    /// which is then reused for the remaining addresses. A real deployment
    /// must use a fixed port.
    pub port: u16,
    /// Interface addresses to listen on. Must not be empty. The wildcard
    /// addresses `0.0.0.0` / `::` listen on every interface of their family
    /// and collapse the other addresses of that family; exact duplicates are
    /// de-duplicated (see [`Self::effective_bind_addresses`]).
    pub bind_addresses: Vec<IpAddr>,
    /// Directory exposed to TFTP clients. Must exist and be a directory.
    pub root_dir: PathBuf,
    /// WRQ uploads. Must remain `false` in this release; enabling it is a
    /// configuration error until uploads have been security-reviewed.
    pub allow_upload: bool,
    /// Allow uploads to replace existing files (requires `allow_upload`).
    pub overwrite: bool,
    /// Largest file served or accepted, in bytes.
    pub max_file_bytes: u64,
    /// Largest block size the server will negotiate (`blksize` cap).
    pub block_size: u16,
    /// Default retransmission timeout in seconds (RFC 1350 baseline).
    pub timeout_secs: u64,
    /// Retransmissions before a transfer fails.
    pub max_retries: u32,
    /// Maximum concurrently active transfers.
    pub max_sessions: usize,
    /// Hard limit on the wall-clock duration of one transfer.
    pub max_transfer_duration_secs: u64,
    /// How long `stop()` waits for active sessions before aborting them.
    pub shutdown_grace_secs: u64,
    /// Number of events kept in the in-memory event ring.
    pub event_buffer: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            bind_addresses: DEFAULT_BIND_ADDRESSES.to_vec(),
            root_dir: PathBuf::from("tftp-root"),
            allow_upload: false,
            overwrite: false,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            block_size: DEFAULT_BLOCK_SIZE,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            max_retries: DEFAULT_MAX_RETRIES,
            max_sessions: DEFAULT_MAX_SESSIONS,
            max_transfer_duration_secs: DEFAULT_MAX_TRANSFER_DURATION_SECS,
            shutdown_grace_secs: DEFAULT_SHUTDOWN_GRACE_SECS,
            event_buffer: DEFAULT_EVENT_BUFFER,
        }
    }
}

impl ServerConfig {
    /// Retransmission timeout.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }

    /// Shutdown grace period.
    #[must_use]
    pub fn shutdown_grace(&self) -> Duration {
        Duration::from_secs(self.shutdown_grace_secs)
    }

    /// Hard per-transfer deadline.
    #[must_use]
    pub fn max_transfer_duration(&self) -> Duration {
        Duration::from_secs(self.max_transfer_duration_secs)
    }

    /// Block size as `usize`.
    #[must_use]
    pub fn max_block_size(&self) -> usize {
        usize::from(self.block_size)
    }

    /// Listen addresses after de-duplication and wildcard collapsing.
    ///
    /// Addresses keep their configured order. An exact duplicate is dropped. If
    /// `0.0.0.0` is configured, every other IPv4 address is redundant and is
    /// dropped (the wildcard already covers it); `::` behaves the same way for
    /// IPv6. This keeps the listener set minimal without rejecting a
    /// configuration that an operator wrote understandably.
    #[must_use]
    pub fn effective_bind_addresses(&self) -> Vec<IpAddr> {
        let has_ipv4_wildcard = self
            .bind_addresses
            .iter()
            .any(|address| matches!(address, IpAddr::V4(ip) if ip.is_unspecified()));
        let has_ipv6_wildcard = self
            .bind_addresses
            .iter()
            .any(|address| matches!(address, IpAddr::V6(ip) if ip.is_unspecified()));
        let mut effective: Vec<IpAddr> = Vec::with_capacity(self.bind_addresses.len());
        for address in &self.bind_addresses {
            if effective.contains(address) {
                continue;
            }
            match address {
                IpAddr::V4(ip) if has_ipv4_wildcard && !ip.is_unspecified() => continue,
                IpAddr::V6(ip) if has_ipv6_wildcard && !ip.is_unspecified() => continue,
                _ => effective.push(*address),
            }
        }
        effective
    }

    /// Configured socket addresses (`effective_bind_addresses` + `port`).
    #[must_use]
    pub fn bind_socket_addrs(&self) -> Vec<SocketAddr> {
        self.effective_bind_addresses()
            .into_iter()
            .map(|address| SocketAddr::new(address, self.port))
            .collect()
    }

    /// Compact operator-facing bind description, for logs and the status bar.
    #[must_use]
    pub fn bind_summary(&self) -> String {
        let addrs = self.bind_socket_addrs();
        if addrs.len() <= 3 {
            return addrs
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
        }
        format!("{} and {} more", addrs[0], addrs.len().saturating_sub(1))
    }

    /// Validate every field, including the root directory's existence.
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] describing the first invalid field.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !(MIN_CONFIG_BLOCK_SIZE..=MAX_CONFIG_BLOCK_SIZE).contains(&self.block_size) {
            return Err(ConfigError::BlockSize(self.block_size));
        }
        if !(1..=255).contains(&self.timeout_secs) {
            return Err(ConfigError::Timeout(self.timeout_secs));
        }
        if !(1..=20).contains(&self.max_retries) {
            return Err(ConfigError::MaxRetries(self.max_retries));
        }
        if !(1..=1024).contains(&self.max_sessions) {
            return Err(ConfigError::MaxSessions(self.max_sessions));
        }
        if !(1..=86_400).contains(&self.max_transfer_duration_secs) {
            return Err(ConfigError::MaxTransferDuration(
                self.max_transfer_duration_secs,
            ));
        }
        if !(1..=60).contains(&self.shutdown_grace_secs) {
            return Err(ConfigError::ShutdownGrace(self.shutdown_grace_secs));
        }
        if !(16..=65_536).contains(&self.event_buffer) {
            return Err(ConfigError::EventBuffer(self.event_buffer));
        }
        if self.max_file_bytes == 0 {
            return Err(ConfigError::MaxFileBytes);
        }
        if self.allow_upload {
            return Err(ConfigError::UploadNotSupported);
        }
        if self.overwrite {
            return Err(ConfigError::OverwriteNotSupported);
        }
        if self.bind_addresses.is_empty() {
            return Err(ConfigError::NoBindAddresses);
        }
        if self.bind_addresses.len() > MAX_BIND_ADDRESSES {
            return Err(ConfigError::TooManyBindAddresses(self.bind_addresses.len()));
        }
        for address in &self.bind_addresses {
            if address.is_multicast() || matches!(address, IpAddr::V4(ip) if ip.is_broadcast()) {
                return Err(ConfigError::BindNotUnicast(*address));
            }
        }
        let metadata = std::fs::metadata(&self.root_dir).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => ConfigError::RootMissing(self.root_dir.clone()),
            _ => ConfigError::RootUnreadable(self.root_dir.clone(), error),
        })?;
        if !metadata.is_dir() {
            return Err(ConfigError::RootNotDirectory(self.root_dir.clone()));
        }
        Ok(())
    }

    /// Canonicalise the root directory (strips `.`/`..` and resolves links in
    /// the root itself). The sandbox rejects links *inside* the root.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::RootUnresolvable`] when the path cannot be resolved.
    pub fn canonical_root(&self) -> Result<PathBuf, ConfigError> {
        std::fs::canonicalize(&self.root_dir)
            .map_err(|error| ConfigError::RootUnresolvable(self.root_dir.clone(), error))
    }

    /// Parse and validate a TOML document.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Toml`] for syntax/unknown-field errors and any
    /// validation error from [`Self::validate`].
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    /// Serialise to pretty TOML (used by `print-default-config` and the GUI).
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::TomlSerialise`] if serialisation fails.
    pub fn to_toml_string(&self) -> Result<String, ConfigError> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Human readable root path without the Windows verbatim prefix.
    #[must_use]
    pub fn display_root(&self) -> String {
        display_path(&self.root_dir)
    }
}

/// Strip a Windows `\\?\` verbatim prefix for display purposes only.
#[must_use]
pub fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(stripped) = text.strip_prefix(r"\\?\") {
            return stripped.to_string();
        }
    }
    text.into_owned()
}

/// Configuration errors. Messages are safe to show to operators and in the GUI.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// `block_size` outside the RFC 2348 range.
    #[error(
        "block_size must be between {MIN_CONFIG_BLOCK_SIZE} and {MAX_CONFIG_BLOCK_SIZE}, got {0}"
    )]
    BlockSize(u16),
    /// `timeout_secs` outside 1..=255.
    #[error("timeout_secs must be between 1 and 255, got {0}")]
    Timeout(u64),
    /// `max_retries` outside 1..=20.
    #[error("max_retries must be between 1 and 20, got {0}")]
    MaxRetries(u32),
    /// `max_sessions` outside 1..=1024.
    #[error("max_sessions must be between 1 and 1024, got {0}")]
    MaxSessions(usize),
    /// Transfer duration outside 1..=86400 seconds.
    #[error("max_transfer_duration_secs must be between 1 and 86400, got {0}")]
    MaxTransferDuration(u64),
    /// Shutdown grace outside 1..=60 seconds.
    #[error("shutdown_grace_secs must be between 1 and 60, got {0}")]
    ShutdownGrace(u64),
    /// Event ring size outside 16..=65536.
    #[error("event_buffer must be between 16 and 65536, got {0}")]
    EventBuffer(usize),
    /// `max_file_bytes` must be positive.
    #[error("max_file_bytes must be greater than zero")]
    MaxFileBytes,
    /// Uploads are not implemented in this release.
    #[error("allow_upload=true is not supported: WRQ uploads have not been security-reviewed yet; set allow_upload=false")]
    UploadNotSupported,
    /// Overwrite requires uploads and is not implemented.
    #[error(
        "overwrite=true is not supported: it requires uploads, which are disabled in this release"
    )]
    OverwriteNotSupported,
    /// No listen address was configured.
    #[error("bind_addresses must contain at least one address")]
    NoBindAddresses,
    /// More addresses than the server accepts.
    #[error("bind_addresses supports at most {MAX_BIND_ADDRESSES} addresses, got {0}")]
    TooManyBindAddresses(usize),
    /// Multicast/broadcast bind addresses are not supported.
    #[error("bind address {0} is not a unicast interface address")]
    BindNotUnicast(IpAddr),
    /// Root directory does not exist.
    #[error("root_dir {0:?} does not exist")]
    RootMissing(PathBuf),
    /// Root directory is not a directory.
    #[error("root_dir {0:?} is not a directory")]
    RootNotDirectory(PathBuf),
    /// Root metadata could not be read.
    #[error("root_dir {0:?} cannot be read: {1}")]
    RootUnreadable(PathBuf, std::io::Error),
    /// Root path could not be canonicalised.
    #[error("root_dir {0:?} cannot be resolved: {1}")]
    RootUnresolvable(PathBuf, std::io::Error),
    /// TOML syntax or unknown-field error.
    #[error("invalid configuration TOML: {0}")]
    Toml(#[from] toml::de::Error),
    /// TOML serialisation error.
    #[error("could not serialise configuration: {0}")]
    TomlSerialise(#[from] toml::ser::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_root() -> (tempfile::TempDir, ServerConfig) {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = ServerConfig {
            root_dir: dir.path().to_path_buf(),
            ..ServerConfig::default()
        };
        (dir, config)
    }

    #[test]
    fn default_config_is_safe() {
        let config = ServerConfig::default();
        assert_eq!(config.block_size, 512);
        assert_eq!(config.max_sessions, DEFAULT_MAX_SESSIONS);
        assert!(!config.allow_upload);
        assert!(!config.overwrite);
        assert_eq!(config.port, 69);
        assert_eq!(
            config.bind_addresses,
            vec![IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)]
        );
        assert!(crate::error::is_privileged_port(config.port));
        assert!(config.effective_bind_addresses()[0].is_loopback());
    }

    #[test]
    fn valid_config_roundtrips_through_toml() {
        let (_dir, mut config) = temp_root();
        config.port = 6969;
        config.bind_addresses = vec![
            "0.0.0.0".parse().expect("addr"),
            "::1".parse().expect("addr"),
        ];
        config.max_sessions = 7;
        let text = config.to_toml_string().expect("serialise");
        assert!(text.contains("bind_addresses"), "{text}");
        assert!(text.contains("\"0.0.0.0\""), "{text}");
        assert!(text.contains("\"::1\""), "{text}");
        let parsed = ServerConfig::from_toml_str(&text).expect("parse");
        assert_eq!(parsed, config);
    }

    #[test]
    fn unknown_and_legacy_fields_are_rejected() {
        let unknown = r#"
bind_addresses = ["127.0.0.1"]
root_dir = "does-not-matter"
surprise = true
"#;
        assert!(matches!(
            ServerConfig::from_toml_str(unknown),
            Err(ConfigError::Toml(_))
        ));

        let legacy = "bind_address = \"127.0.0.1:6969\"\nroot_dir = \"does-not-matter\"\n";
        let error = ServerConfig::from_toml_str(legacy).expect_err("old key must not be accepted");
        let message = error.to_string();
        assert!(
            message.contains("bind_address"),
            "the error should name the rejected key: {message}"
        );
    }

    #[test]
    fn missing_fields_use_safe_defaults() {
        let (dir, _config) = temp_root();
        let text = format!("root_dir = {:?}\n", dir.path().to_string_lossy());
        let parsed = ServerConfig::from_toml_str(&text).expect("parse");
        assert_eq!(parsed.block_size, 512);
        assert!(!parsed.allow_upload);
        assert!(!parsed.overwrite);
        assert_eq!(parsed.port, 69);
        assert_eq!(
            parsed.bind_addresses,
            vec![IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)]
        );
    }

    #[test]
    fn bind_addresses_are_validated() {
        let (_dir, mut config) = temp_root();
        config.bind_addresses = Vec::new();
        assert!(matches!(
            config.validate(),
            Err(ConfigError::NoBindAddresses)
        ));

        config.bind_addresses =
            vec![IpAddr::V4(std::net::Ipv4Addr::LOCALHOST); MAX_BIND_ADDRESSES + 1];
        assert!(matches!(
            config.validate(),
            Err(ConfigError::TooManyBindAddresses(_))
        ));

        config.bind_addresses = vec!["224.0.0.1".parse().expect("ip")];
        assert!(matches!(
            config.validate(),
            Err(ConfigError::BindNotUnicast(_))
        ));

        config.bind_addresses = vec!["255.255.255.255".parse().expect("ip")];
        assert!(matches!(
            config.validate(),
            Err(ConfigError::BindNotUnicast(_))
        ));

        config.bind_addresses = vec!["ff02::1".parse().expect("ip")];
        assert!(matches!(
            config.validate(),
            Err(ConfigError::BindNotUnicast(_))
        ));
    }

    #[test]
    fn wildcard_addresses_collapse_their_family_and_duplicates_are_dropped() {
        let (_dir, mut config) = temp_root();
        config.bind_addresses = vec![
            "127.0.0.1".parse().expect("ip"),
            "192.168.1.5".parse().expect("ip"),
            "::1".parse().expect("ip"),
        ];
        assert_eq!(config.effective_bind_addresses(), config.bind_addresses);

        config.bind_addresses = vec![
            "192.168.1.5".parse().expect("ip"),
            "0.0.0.0".parse().expect("ip"),
            "127.0.0.1".parse().expect("ip"),
            "::1".parse().expect("ip"),
            "0.0.0.0".parse().expect("ip"),
        ];
        let effective: Vec<IpAddr> = config.effective_bind_addresses();
        assert_eq!(
            effective,
            vec![
                "0.0.0.0".parse::<IpAddr>().expect("ip"),
                "::1".parse::<IpAddr>().expect("ip")
            ]
        );
    }

    #[test]
    fn bind_socket_addrs_and_summary_use_the_shared_port() {
        let (_dir, mut config) = temp_root();
        config.port = 6969;
        config.bind_addresses = vec![
            "127.0.0.1".parse().expect("ip"),
            "192.168.1.5".parse().expect("ip"),
        ];
        assert_eq!(
            config.bind_socket_addrs(),
            vec![
                "127.0.0.1:6969".parse().expect("addr"),
                "192.168.1.5:6969".parse().expect("addr")
            ]
        );
        assert_eq!(config.bind_summary(), "127.0.0.1:6969, 192.168.1.5:6969");

        config.bind_addresses = (1..=4)
            .map(|octet| IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, octet)))
            .collect();
        assert_eq!(config.bind_summary(), "127.0.0.1:6969 and 3 more");
    }

    #[test]
    fn rejects_out_of_range_values() {
        let (_dir, mut config) = temp_root();
        config.timeout_secs = 0;
        assert!(matches!(config.validate(), Err(ConfigError::Timeout(0))));
        config.timeout_secs = 3;

        config.block_size = 7;
        assert!(matches!(config.validate(), Err(ConfigError::BlockSize(7))));
        config.block_size = 512;

        config.max_retries = 0;
        assert!(matches!(config.validate(), Err(ConfigError::MaxRetries(0))));
        config.max_retries = 3;

        config.max_sessions = 0;
        assert!(matches!(
            config.validate(),
            Err(ConfigError::MaxSessions(0))
        ));
        config.max_sessions = 2;

        config.event_buffer = 1;
        assert!(matches!(
            config.validate(),
            Err(ConfigError::EventBuffer(1))
        ));
        config.event_buffer = 64;

        config.max_file_bytes = 0;
        assert!(matches!(config.validate(), Err(ConfigError::MaxFileBytes)));
    }

    #[test]
    fn rejects_uploads_until_implemented() {
        let (_dir, mut config) = temp_root();
        config.allow_upload = true;
        assert!(matches!(
            config.validate(),
            Err(ConfigError::UploadNotSupported)
        ));

        config.allow_upload = false;
        config.overwrite = true;
        assert!(matches!(
            config.validate(),
            Err(ConfigError::OverwriteNotSupported)
        ));
    }

    #[test]
    fn rejects_missing_or_non_directory_root() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = ServerConfig {
            root_dir: dir.path().join("missing"),
            ..ServerConfig::default()
        };
        assert!(matches!(
            missing.validate(),
            Err(ConfigError::RootMissing(_))
        ));

        let file_root = ServerConfig {
            root_dir: dir.path().join("file"),
            ..ServerConfig::default()
        };
        fs::write(&file_root.root_dir, b"x").expect("write file");
        assert!(matches!(
            file_root.validate(),
            Err(ConfigError::RootNotDirectory(_))
        ));
    }

    #[test]
    fn canonical_root_strips_dot_components() {
        let (_dir, mut config) = temp_root();
        config.root_dir = config.root_dir.join(".");
        let canonical = config.canonical_root().expect("canonical");
        assert!(canonical.is_absolute());
    }

    #[test]
    fn display_path_hides_verbatim_prefix_on_windows() {
        let (_dir, config) = temp_root();
        let display = config.display_root();
        assert!(
            !display.contains(r"\\?\"),
            "unexpected verbatim prefix in {display}"
        );
    }
}

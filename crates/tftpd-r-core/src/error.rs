//! Shared error types: TFTP error codes plus codec/server errors.

use std::net::SocketAddr;

/// TFTP ERROR packet codes (RFC 1350 section 5, RFC 2347).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ErrorCode {
    /// Not defined, see error message (if any).
    Undefined = 0,
    /// File not found.
    FileNotFound = 1,
    /// Access violation.
    AccessViolation = 2,
    /// Disk full or allocation exceeded.
    DiskFull = 3,
    /// Illegal TFTP operation.
    IllegalOperation = 4,
    /// Unknown transfer ID.
    UnknownTransferId = 5,
    /// File already exists.
    FileAlreadyExists = 6,
    /// No such user.
    NoSuchUser = 7,
    /// Option negotiation failed (RFC 2347).
    OptionNegotiation = 8,
}

impl ErrorCode {
    /// Numeric on-the-wire value.
    #[must_use]
    pub fn as_u16(self) -> u16 {
        self as u16
    }

    /// Map a wire value, returning `None` for reserved/unknown codes.
    #[must_use]
    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            0 => Some(Self::Undefined),
            1 => Some(Self::FileNotFound),
            2 => Some(Self::AccessViolation),
            3 => Some(Self::DiskFull),
            4 => Some(Self::IllegalOperation),
            5 => Some(Self::UnknownTransferId),
            6 => Some(Self::FileAlreadyExists),
            7 => Some(Self::NoSuchUser),
            8 => Some(Self::OptionNegotiation),
            _ => None,
        }
    }

    /// Short, client-safe default message. Never contains local paths.
    #[must_use]
    pub fn default_message(self) -> &'static str {
        match self {
            Self::Undefined => "Transfer failed",
            Self::FileNotFound => "File not found",
            Self::AccessViolation => "Access violation",
            Self::DiskFull => "Disk full",
            Self::IllegalOperation => "Illegal TFTP operation",
            Self::UnknownTransferId => "Unknown transfer ID",
            Self::FileAlreadyExists => "File already exists",
            Self::NoSuchUser => "No such user",
            Self::OptionNegotiation => "Option negotiation failed",
        }
    }
}

/// Errors produced while decoding a datagram. They are purely syntactic: no
/// file system or network state is inspected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// Datagram ends before a required field.
    #[error("truncated packet")]
    Truncated,
    /// Opcode is not one of RRQ/WRQ/DATA/ACK/ERROR/OACK.
    #[error("unknown opcode {0}")]
    UnknownOpcode(u16),
    /// Structurally invalid packet (missing terminator, empty field, ...).
    #[error("malformed packet: {0}")]
    Malformed(&'static str),
    /// A known option carried an invalid or duplicated value.
    #[error("invalid option: {0}")]
    InvalidOption(&'static str),
    /// The transfer mode is not supported by this server.
    #[error("unsupported transfer mode: {0}")]
    UnsupportedMode(String),
    /// DATA payload or request datagram exceeds the configured limit.
    #[error("datagram exceeds the allowed size")]
    TooLarge,
    /// Text fields must be valid UTF-8.
    #[error("packet contains non-UTF-8 text")]
    InvalidUtf8,
}

/// Errors produced while encoding a packet (should not happen for packets the
/// server constructs; mostly a defensive check for NUL bytes in text fields).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    /// A text field contains an interior NUL byte.
    #[error("text field contains an interior NUL byte")]
    InteriorNul,
    /// A DATA payload does not fit in `u16` length accounting.
    #[error("payload too large")]
    PayloadTooLarge,
}

/// Lowest port number that requires administrator/root privileges.
pub const FIRST_PRIVILEGED_PORT: u16 = 1;
/// Highest privileged (well-known) port number.
pub const LAST_PRIVILEGED_PORT: u16 = 1023;

/// Fatal errors returned when starting the server.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// The configuration failed validation.
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    /// The sandbox root directory is unusable.
    #[error(transparent)]
    Sandbox(#[from] crate::filesystem::SandboxError),
    /// The UDP listening socket could not be bound.
    #[error("failed to bind UDP socket on {addr}: {source}")]
    Bind {
        /// Requested bind address.
        addr: SocketAddr,
        /// Underlying OS error.
        source: std::io::Error,
    },
    /// A privileged port could not be bound because the process lacks rights.
    ///
    /// This is a distinct variant so the GUI and the daemon can show an
    /// actionable, non-forcing hint instead of a generic bind failure. The
    /// server never retries on another port and never elevates itself.
    #[error(
        "cannot bind UDP {addr}: permission denied; port {port} is privileged and needs administrator/root privileges. Run with sufficient privileges or choose an unprivileged port such as 6969 (the configuration is not changed automatically)",
        port = .addr.port()
    )]
    PrivilegedPort {
        /// Requested bind address.
        addr: SocketAddr,
        /// Underlying OS error.
        source: std::io::Error,
    },
    /// A per-transfer socket could not be created.
    #[error("failed to create transfer socket: {0}")]
    TransferSocket(std::io::Error),
}

impl ServerError {
    /// Classify a bind failure.
    ///
    /// A permission error on a privileged port becomes
    /// [`ServerError::PrivilegedPort`] so front-ends can suggest an
    /// unprivileged port; everything else stays a plain [`ServerError::Bind`].
    #[must_use]
    pub fn bind(addr: SocketAddr, source: std::io::Error) -> Self {
        if is_privileged_port(addr.port()) && is_permission_denied(&source) {
            Self::PrivilegedPort { addr, source }
        } else {
            Self::Bind { addr, source }
        }
    }
}

/// Whether a port needs elevated privileges to bind.
#[must_use]
pub fn is_privileged_port(port: u16) -> bool {
    (FIRST_PRIVILEGED_PORT..=LAST_PRIVILEGED_PORT).contains(&port)
}

/// Whether an OS error means "this process may not bind that address".
///
/// `std` normally maps `EACCES`/`WSAEACCES` to `PermissionDenied`; the raw
/// codes are checked as well so the classification does not depend on the
/// platform's error mapping.
fn is_permission_denied(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        return true;
    }
    match error.raw_os_error() {
        #[cfg(windows)]
        Some(10013) => true, // WSAEACCES
        #[cfg(unix)]
        Some(13) => true, // EACCES
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    fn addr(text: &str) -> SocketAddr {
        text.parse().expect("socket address")
    }

    #[test]
    fn permission_denied_on_port_69_gets_an_actionable_hint() {
        let error = ServerError::bind(
            addr("127.0.0.1:69"),
            std::io::Error::from(ErrorKind::PermissionDenied),
        );
        assert!(matches!(error, ServerError::PrivilegedPort { .. }));
        let text = error.to_string();
        assert!(text.contains("administrator"), "missing hint: {text}");
        assert!(text.contains("6969"), "missing port suggestion: {text}");
        assert!(
            text.contains("not changed automatically"),
            "the hint must not imply an automatic fallback: {text}"
        );
    }

    #[test]
    fn permission_denied_on_a_high_port_stays_a_plain_bind_error() {
        let error = ServerError::bind(
            addr("127.0.0.1:6969"),
            std::io::Error::from(ErrorKind::PermissionDenied),
        );
        assert!(matches!(error, ServerError::Bind { .. }));
        assert!(!error.to_string().contains("privileged"));
    }

    #[test]
    fn other_errors_on_a_privileged_port_stay_plain_bind_errors() {
        let error = ServerError::bind(
            addr("127.0.0.1:69"),
            std::io::Error::from(ErrorKind::AddrInUse),
        );
        assert!(matches!(error, ServerError::Bind { .. }));
        assert!(!error.to_string().contains("privileged"));
    }

    #[test]
    fn privileged_port_range_is_one_to_1023() {
        assert!(is_privileged_port(1));
        assert!(is_privileged_port(69));
        assert!(is_privileged_port(1023));
        assert!(!is_privileged_port(0));
        assert!(!is_privileged_port(1024));
        assert!(!is_privileged_port(6969));
    }
}

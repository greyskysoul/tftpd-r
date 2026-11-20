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
    /// A per-transfer socket could not be created.
    #[error("failed to create transfer socket: {0}")]
    TransferSocket(std::io::Error),
}

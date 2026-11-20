//! Packet data types shared by the codec, the session state machine and tests.

use crate::error::ErrorCode;

/// Largest DATA payload defined by RFC 2348 (`blksize` 65464).
pub const MAX_BLOCK_SIZE: usize = 65_464;

/// Baseline DATA payload size (RFC 1350).
pub const DEFAULT_BLOCK_SIZE: usize = 512;

/// Negotiated option list. Order is preserved so an OACK echoes the client's
/// option order, which some clients rely on.
pub type Options = Vec<(String, String)>;

/// Transfer modes recognised on the wire.
///
/// Only [`Mode::Octet`] is served; the other values are parsed so the server can
/// reject them explicitly instead of silently treating them as `octet`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Raw 8-bit transfer (supported).
    Octet,
    /// ASCII line-ending translation (not supported, rejected).
    NetAscii,
    /// Obsolete mail mode (not supported, rejected).
    Mail,
}

impl Mode {
    /// Parse a mode string case-insensitively. Returns `None` for unknown modes.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        if raw.eq_ignore_ascii_case("octet") {
            Some(Self::Octet)
        } else if raw.eq_ignore_ascii_case("netascii") {
            Some(Self::NetAscii)
        } else if raw.eq_ignore_ascii_case("mail") {
            Some(Self::Mail)
        } else {
            None
        }
    }

    /// Canonical wire spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Octet => "octet",
            Self::NetAscii => "netascii",
            Self::Mail => "mail",
        }
    }
}

/// A read or write request (RRQ/WRQ share the same layout).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Client-supplied file name, exactly as received (may contain `/`).
    pub filename: String,
    /// Transfer mode.
    pub mode: Mode,
    /// Ordered option name/value pairs (names are lower-case).
    pub options: Options,
}

/// A decoded TFTP packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    /// Read request.
    Rrq(Request),
    /// Write request.
    Wrq(Request),
    /// Data block (`block` starts at 1).
    Data {
        /// Block number.
        block: u16,
        /// Payload bytes (0..=negotiated block size).
        data: Vec<u8>,
    },
    /// Acknowledgement for a block number.
    Ack {
        /// Acknowledged block number.
        block: u16,
    },
    /// Error packet.
    Error {
        /// Error code.
        code: ErrorCode,
        /// Human readable, client-safe message.
        message: String,
    },
    /// Option acknowledgement (RFC 2347).
    Oack(Options),
}

impl Packet {
    /// Opcode that will be written on the wire.
    #[must_use]
    pub fn opcode(&self) -> u16 {
        match self {
            Self::Rrq(_) => 1,
            Self::Wrq(_) => 2,
            Self::Data { .. } => 3,
            Self::Ack { .. } => 4,
            Self::Error { .. } => 5,
            Self::Oack(_) => 6,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_parsing_is_case_insensitive() {
        assert_eq!(Mode::parse("octet"), Some(Mode::Octet));
        assert_eq!(Mode::parse("OCTET"), Some(Mode::Octet));
        assert_eq!(Mode::parse("NetAscii"), Some(Mode::NetAscii));
        assert_eq!(Mode::parse("mail"), Some(Mode::Mail));
        assert_eq!(Mode::parse("binary"), None);
    }

    #[test]
    fn opcodes_match_rfc() {
        assert_eq!(
            Packet::Rrq(Request {
                filename: "a".into(),
                mode: Mode::Octet,
                options: vec![]
            })
            .opcode(),
            1
        );
        assert_eq!(
            Packet::Wrq(Request {
                filename: "a".into(),
                mode: Mode::Octet,
                options: vec![]
            })
            .opcode(),
            2
        );
        assert_eq!(
            Packet::Data {
                block: 1,
                data: vec![]
            }
            .opcode(),
            3
        );
        assert_eq!(Packet::Ack { block: 1 }.opcode(), 4);
        assert_eq!(
            Packet::Error {
                code: ErrorCode::Undefined,
                message: String::new()
            }
            .opcode(),
            5
        );
        assert_eq!(Packet::Oack(vec![]).opcode(), 6);
    }

    #[test]
    fn mode_wire_names_are_stable() {
        assert_eq!(Mode::Octet.as_str(), "octet");
        assert_eq!(Mode::NetAscii.as_str(), "netascii");
        assert_eq!(Mode::Mail.as_str(), "mail");
    }
}

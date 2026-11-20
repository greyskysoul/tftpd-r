//! RFC 2347/2348/2349 option negotiation.
//!
//! Negotiation is a pure function so the accepted range, clamping and OACK
//! contents can be tested without a socket.

use std::time::Duration;

use crate::error::ErrorCode;

/// Smallest legal `blksize` value (RFC 2348).
pub const MIN_BLOCK_SIZE: usize = 8;
/// Largest legal `blksize` value (RFC 2348).
pub const MAX_BLOCK_SIZE: usize = 65_464;
/// Smallest legal `timeout` value in seconds (RFC 2349).
pub const MIN_TIMEOUT_SECS: u64 = 1;
/// Largest legal `timeout` value in seconds (RFC 2349).
pub const MAX_TIMEOUT_SECS: u64 = 255;

/// Result of option negotiation for one transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Negotiated {
    /// Block size to use for DATA payloads.
    pub block_size: usize,
    /// Retransmission timeout requested/accepted by the client.
    pub timeout: Duration,
    /// Size to report in the OACK (`tsize`), when requested.
    pub transfer_size: Option<u64>,
    /// Options to echo in the OACK, in request order.
    pub oack: Vec<(String, String)>,
}

impl Negotiated {
    /// Whether an OACK must be sent before the first DATA block.
    #[must_use]
    pub fn needs_oack(&self) -> bool {
        !self.oack.is_empty()
    }
}

/// A malformed or unsupported option request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OptionError {
    /// `blksize` outside 8..=65464 or not a decimal number.
    #[error("invalid blksize option")]
    BlockSize,
    /// `timeout` outside 1..=255 or not a decimal number.
    #[error("invalid timeout option")]
    Timeout,
    /// `tsize` not a decimal number.
    #[error("invalid tsize option")]
    TransferSize,
    /// The same option was requested twice.
    #[error("duplicate option {0}")]
    Duplicate(String),
}

impl OptionError {
    /// TFTP error code for this failure.
    #[must_use]
    pub fn error_code(&self) -> ErrorCode {
        ErrorCode::OptionNegotiation
    }
}

/// Negotiate options for an RRQ.
///
/// Unknown options are ignored (RFC 2347). Out-of-range or malformed values for
/// known options fail the negotiation, which the caller reports as ERROR 8.
///
/// # Errors
///
/// Returns [`OptionError`] when a recognised option is malformed, out of range
/// or duplicated.
pub fn negotiate(
    requested: &[(String, String)],
    server_max_block_size: usize,
    default_timeout: Duration,
    file_size: Option<u64>,
) -> Result<Negotiated, OptionError> {
    let mut negotiated = Negotiated {
        block_size: 512,
        timeout: default_timeout,
        transfer_size: None,
        oack: Vec::new(),
    };
    let cap = server_max_block_size.clamp(MIN_BLOCK_SIZE, MAX_BLOCK_SIZE);
    let mut seen: Vec<String> = Vec::new();
    for (name, value) in requested {
        let lowercase = name.to_ascii_lowercase();
        if seen.iter().any(|previous| previous == &lowercase) {
            return Err(OptionError::Duplicate(lowercase));
        }
        seen.push(lowercase.clone());
        match lowercase.as_str() {
            "blksize" => {
                let requested_size = parse_decimal(value).ok_or(OptionError::BlockSize)?;
                if !(MIN_BLOCK_SIZE as u64..=MAX_BLOCK_SIZE as u64).contains(&requested_size) {
                    return Err(OptionError::BlockSize);
                }
                let chosen = usize::try_from(requested_size)
                    .unwrap_or(MAX_BLOCK_SIZE)
                    .min(cap);
                negotiated.block_size = chosen;
                negotiated
                    .oack
                    .push(("blksize".to_string(), chosen.to_string()));
            }
            "timeout" => {
                let seconds = parse_decimal(value).ok_or(OptionError::Timeout)?;
                if !(MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS).contains(&seconds) {
                    return Err(OptionError::Timeout);
                }
                negotiated.timeout = Duration::from_secs(seconds);
                negotiated
                    .oack
                    .push(("timeout".to_string(), seconds.to_string()));
            }
            "tsize" => {
                let _requested_size = parse_decimal(value).ok_or(OptionError::TransferSize)?;
                if let Some(size) = file_size {
                    negotiated.transfer_size = Some(size);
                    negotiated
                        .oack
                        .push(("tsize".to_string(), size.to_string()));
                }
            }
            // Unknown options are ignored, never echoed back.
            _ => {}
        }
    }
    Ok(negotiated)
}

fn parse_decimal(value: &str) -> Option<u64> {
    if value.is_empty() || value.len() > 20 {
        return None;
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn defaults_when_no_options_requested() {
        let negotiated = negotiate(&[], 512, Duration::from_secs(3), Some(100)).expect("negotiate");
        assert_eq!(negotiated.block_size, 512);
        assert_eq!(negotiated.timeout, Duration::from_secs(3));
        assert_eq!(negotiated.transfer_size, None);
        assert!(!negotiated.needs_oack());
    }

    #[test]
    fn accepts_blksize_and_clamps_to_server_cap() {
        let negotiated = negotiate(
            &opts(&[("blksize", "1468")]),
            1024,
            Duration::from_secs(3),
            Some(10),
        )
        .expect("negotiate");
        assert_eq!(negotiated.block_size, 1024);
        assert_eq!(
            negotiated.oack,
            vec![("blksize".to_string(), "1024".to_string())]
        );
        assert!(negotiated.needs_oack());
    }

    #[test]
    fn accepts_full_blksize_range() {
        for size in [8u64, 512, 65_464] {
            let value = size.to_string();
            let negotiated = negotiate(
                &opts(&[("blksize", value.as_str())]),
                65_464,
                Duration::from_secs(3),
                Some(10),
            )
            .expect("negotiate");
            assert_eq!(negotiated.block_size as u64, size);
        }
    }

    #[test]
    fn rejects_out_of_range_blksize() {
        assert_eq!(
            negotiate(
                &opts(&[("blksize", "7")]),
                512,
                Duration::from_secs(3),
                Some(1)
            ),
            Err(OptionError::BlockSize)
        );
        assert_eq!(
            negotiate(
                &opts(&[("blksize", "65465")]),
                512,
                Duration::from_secs(3),
                Some(1)
            ),
            Err(OptionError::BlockSize)
        );
        assert_eq!(
            negotiate(
                &opts(&[("blksize", "abc")]),
                512,
                Duration::from_secs(3),
                Some(1)
            ),
            Err(OptionError::BlockSize)
        );
    }

    #[test]
    fn accepts_timeout_and_uses_it() {
        let negotiated = negotiate(
            &opts(&[("timeout", "7")]),
            512,
            Duration::from_secs(3),
            Some(1),
        )
        .expect("negotiate");
        assert_eq!(negotiated.timeout, Duration::from_secs(7));
        assert_eq!(
            negotiated.oack,
            vec![("timeout".to_string(), "7".to_string())]
        );
    }

    #[test]
    fn rejects_bad_timeouts() {
        assert_eq!(
            negotiate(
                &opts(&[("timeout", "0")]),
                512,
                Duration::from_secs(3),
                Some(1)
            ),
            Err(OptionError::Timeout)
        );
        assert_eq!(
            negotiate(
                &opts(&[("timeout", "256")]),
                512,
                Duration::from_secs(3),
                Some(1)
            ),
            Err(OptionError::Timeout)
        );
    }

    #[test]
    fn tsize_reports_file_size() {
        let negotiated = negotiate(
            &opts(&[("tsize", "0")]),
            512,
            Duration::from_secs(3),
            Some(1234),
        )
        .expect("negotiate");
        assert_eq!(negotiated.transfer_size, Some(1234));
        assert_eq!(
            negotiated.oack,
            vec![("tsize".to_string(), "1234".to_string())]
        );
    }

    #[test]
    fn tsize_is_ignored_when_no_size_is_known() {
        let negotiated = negotiate(&opts(&[("tsize", "0")]), 512, Duration::from_secs(3), None)
            .expect("negotiate");
        assert_eq!(negotiated.transfer_size, None);
        assert!(negotiated.oack.is_empty());
    }

    #[test]
    fn unknown_options_are_ignored() {
        let negotiated = negotiate(
            &opts(&[("windowsize", "4")]),
            512,
            Duration::from_secs(3),
            Some(1),
        )
        .expect("negotiate");
        assert!(!negotiated.needs_oack());
    }

    #[test]
    fn duplicate_options_are_rejected_case_insensitively() {
        assert_eq!(
            negotiate(
                &opts(&[("blksize", "512"), ("BLKSIZE", "1024")]),
                512,
                Duration::from_secs(3),
                Some(1)
            ),
            Err(OptionError::Duplicate("blksize".into()))
        );
    }

    #[test]
    fn error_code_is_negotiation_failure() {
        assert_eq!(
            OptionError::BlockSize.error_code(),
            ErrorCode::OptionNegotiation
        );
    }

    #[test]
    fn decimal_parser_is_strict() {
        assert_eq!(parse_decimal("0"), Some(0));
        assert_eq!(parse_decimal("+8"), None);
        assert_eq!(parse_decimal("-1"), None);
        assert_eq!(parse_decimal(" 8"), None);
        assert_eq!(parse_decimal(""), None);
        assert_eq!(parse_decimal("999999999999999999999"), None);
    }
}

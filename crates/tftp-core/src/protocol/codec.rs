//! Stateless TFTP datagram encoder/decoder.
//!
//! All functions here are pure: they never touch sockets or the file system,
//! which makes the boundary cases easy to test exhaustively.

use crate::error::{DecodeError, EncodeError, ErrorCode};
use crate::protocol::packet::{Mode, Options, Packet, Request, MAX_BLOCK_SIZE};

/// Upper bound for a request (RRQ/WRQ) datagram including options.
///
/// RFC 1350 requests are tiny; RFC 2347 options add a few dozen bytes. A hard
/// limit keeps a malicious client from forcing large allocations.
pub const MAX_REQUEST_BYTES: usize = 1024;

/// Maximum length of a file name inside a request.
pub const MAX_FILENAME_BYTES: usize = 512;

/// Maximum length of an ERROR message.
pub const MAX_ERROR_MESSAGE_BYTES: usize = 512;

const OP_RRQ: u16 = 1;
const OP_WRQ: u16 = 2;
const OP_DATA: u16 = 3;
const OP_ACK: u16 = 4;
const OP_ERROR: u16 = 5;
const OP_OACK: u16 = 6;

/// Decode one datagram.
///
/// `max_block_size` bounds the DATA payload (normally the negotiated block
/// size); requests are additionally bounded by [`MAX_REQUEST_BYTES`].
///
/// # Errors
///
/// Returns [`DecodeError`] for truncated, malformed, oversized or unknown
/// packets. The caller maps these to the appropriate TFTP ERROR response.
pub fn decode(datagram: &[u8], max_block_size: usize) -> Result<Packet, DecodeError> {
    if datagram.len() < 2 {
        return Err(DecodeError::Truncated);
    }
    let opcode = u16::from_be_bytes([datagram[0], datagram[1]]);
    let body = &datagram[2..];
    match opcode {
        OP_RRQ => decode_request(body, false),
        OP_WRQ => decode_request(body, true),
        OP_DATA => decode_data(body, max_block_size),
        OP_ACK => {
            if body.len() != 2 {
                return Err(DecodeError::Malformed("ACK must be exactly 4 bytes"));
            }
            Ok(Packet::Ack {
                block: u16::from_be_bytes([body[0], body[1]]),
            })
        }
        OP_ERROR => decode_error(body),
        OP_OACK => decode_oack(body),
        other => Err(DecodeError::UnknownOpcode(other)),
    }
}

/// Encode a packet into a fresh buffer.
///
/// # Errors
///
/// Returns [`EncodeError::InteriorNul`] if a text field contains a NUL byte and
/// [`EncodeError::PayloadTooLarge`] if a DATA payload exceeds the RFC limit.
pub fn encode(packet: &Packet) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::with_capacity(64);
    encode_into(packet, &mut out)?;
    Ok(out)
}

/// Encode a packet into an existing buffer, clearing it first.
///
/// # Errors
///
/// See [`encode`].
pub fn encode_into(packet: &Packet, out: &mut Vec<u8>) -> Result<(), EncodeError> {
    out.clear();
    match packet {
        Packet::Rrq(request) => encode_request(OP_RRQ, request, out)?,
        Packet::Wrq(request) => encode_request(OP_WRQ, request, out)?,
        Packet::Data { block, data } => {
            if data.len() > MAX_BLOCK_SIZE {
                return Err(EncodeError::PayloadTooLarge);
            }
            out.extend_from_slice(&OP_DATA.to_be_bytes());
            out.extend_from_slice(&block.to_be_bytes());
            out.extend_from_slice(data);
        }
        Packet::Ack { block } => {
            out.extend_from_slice(&OP_ACK.to_be_bytes());
            out.extend_from_slice(&block.to_be_bytes());
        }
        Packet::Error { code, message } => {
            let message = truncate_text(message, MAX_ERROR_MESSAGE_BYTES);
            if message.as_bytes().contains(&0) {
                return Err(EncodeError::InteriorNul);
            }
            out.extend_from_slice(&OP_ERROR.to_be_bytes());
            out.extend_from_slice(&code.as_u16().to_be_bytes());
            out.extend_from_slice(message.as_bytes());
            out.push(0);
        }
        Packet::Oack(options) => {
            out.extend_from_slice(&OP_OACK.to_be_bytes());
            for (name, value) in options {
                push_cstr(name, out)?;
                push_cstr(value, out)?;
            }
        }
    }
    Ok(())
}

/// Replace control characters with spaces and truncate to `max` bytes on a
/// UTF-8 boundary. Used for ERROR messages shown to remote clients.
#[must_use]
pub fn sanitize_text(text: &str, max: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max));
    for ch in text.chars() {
        if out.len() + ch.len_utf8() > max {
            break;
        }
        if ch.is_control() {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

fn truncate_text(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

fn decode_request(body: &[u8], is_write: bool) -> Result<Packet, DecodeError> {
    if body.len() > MAX_REQUEST_BYTES {
        return Err(DecodeError::TooLarge);
    }
    let (filename_bytes, rest) = take_cstr(body)?;
    if filename_bytes.is_empty() {
        return Err(DecodeError::Malformed("empty file name"));
    }
    if filename_bytes.len() > MAX_FILENAME_BYTES {
        return Err(DecodeError::Malformed("file name too long"));
    }
    let filename = std::str::from_utf8(filename_bytes)
        .map_err(|_| DecodeError::InvalidUtf8)?
        .to_string();
    let (mode_bytes, rest) = take_cstr(rest)?;
    if mode_bytes.is_empty() {
        return Err(DecodeError::Malformed("empty transfer mode"));
    }
    let mode_text = std::str::from_utf8(mode_bytes).map_err(|_| DecodeError::InvalidUtf8)?;
    let mode = Mode::parse(mode_text)
        .ok_or_else(|| DecodeError::UnsupportedMode(mode_text.to_string()))?;
    let options = decode_options(rest)?;
    let request = Request {
        filename,
        mode,
        options,
    };
    Ok(if is_write {
        Packet::Wrq(request)
    } else {
        Packet::Rrq(request)
    })
}

fn decode_options(mut rest: &[u8]) -> Result<Options, DecodeError> {
    let mut options = Options::new();
    while !rest.is_empty() {
        let (name_bytes, next) = take_cstr(rest)?;
        if name_bytes.is_empty() {
            return Err(DecodeError::Malformed("empty option name"));
        }
        let (value_bytes, next) = take_cstr(next)?;
        if value_bytes.is_empty() {
            return Err(DecodeError::InvalidOption("empty option value"));
        }
        if name_bytes.len() > 32 || value_bytes.len() > 64 {
            return Err(DecodeError::InvalidOption("option field too long"));
        }
        if !name_bytes.is_ascii() || !value_bytes.is_ascii() {
            return Err(DecodeError::InvalidOption("option fields must be ASCII"));
        }
        let name = std::str::from_utf8(name_bytes)
            .map_err(|_| DecodeError::InvalidUtf8)?
            .to_ascii_lowercase();
        let value = std::str::from_utf8(value_bytes)
            .map_err(|_| DecodeError::InvalidUtf8)?
            .to_string();
        options.push((name, value));
        if options.len() > 16 {
            return Err(DecodeError::InvalidOption("too many options"));
        }
        rest = next;
    }
    Ok(options)
}

fn decode_data(body: &[u8], max_block_size: usize) -> Result<Packet, DecodeError> {
    if body.len() < 2 {
        return Err(DecodeError::Truncated);
    }
    let block = u16::from_be_bytes([body[0], body[1]]);
    let data = &body[2..];
    let limit = max_block_size.clamp(1, MAX_BLOCK_SIZE);
    if data.len() > limit {
        return Err(DecodeError::TooLarge);
    }
    Ok(Packet::Data {
        block,
        data: data.to_vec(),
    })
}

fn decode_error(body: &[u8]) -> Result<Packet, DecodeError> {
    if body.len() < 2 {
        return Err(DecodeError::Truncated);
    }
    let code = u16::from_be_bytes([body[0], body[1]]);
    let message_bytes = match body[2..].split(|byte| *byte == 0).next() {
        Some(bytes) => bytes,
        None => &body[2..],
    };
    if message_bytes.len() > MAX_ERROR_MESSAGE_BYTES {
        return Err(DecodeError::Malformed("error message too long"));
    }
    let message = String::from_utf8_lossy(message_bytes).into_owned();
    Ok(Packet::Error {
        code: ErrorCode::from_u16(code).unwrap_or(ErrorCode::Undefined),
        message,
    })
}

fn decode_oack(body: &[u8]) -> Result<Packet, DecodeError> {
    if body.is_empty() {
        return Err(DecodeError::Malformed("empty OACK"));
    }
    let options = decode_options(body)?;
    if options.is_empty() {
        return Err(DecodeError::Malformed("empty OACK"));
    }
    Ok(Packet::Oack(options))
}

fn take_cstr(input: &[u8]) -> Result<(&[u8], &[u8]), DecodeError> {
    match input.iter().position(|byte| *byte == 0) {
        Some(index) => Ok((&input[..index], &input[index + 1..])),
        None => Err(DecodeError::Malformed("unterminated string field")),
    }
}

fn encode_request(opcode: u16, request: &Request, out: &mut Vec<u8>) -> Result<(), EncodeError> {
    out.extend_from_slice(&opcode.to_be_bytes());
    push_cstr(&request.filename, out)?;
    push_cstr(request.mode.as_str(), out)?;
    for (name, value) in &request.options {
        push_cstr(name, out)?;
        push_cstr(value, out)?;
    }
    Ok(())
}

fn push_cstr(text: &str, out: &mut Vec<u8>) -> Result<(), EncodeError> {
    if text.as_bytes().contains(&0) {
        return Err(EncodeError::InteriorNul);
    }
    out.extend_from_slice(text.as_bytes());
    out.push(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rrq() -> Packet {
        Packet::Rrq(Request {
            filename: "images/boot.img".into(),
            mode: Mode::Octet,
            options: vec![
                ("blksize".into(), "1024".into()),
                ("tsize".into(), "0".into()),
            ],
        })
    }

    #[test]
    fn roundtrip_every_packet_kind() {
        let packets = vec![
            rrq(),
            Packet::Wrq(Request {
                filename: "upload.bin".into(),
                mode: Mode::Octet,
                options: vec![],
            }),
            Packet::Data {
                block: 0,
                data: Vec::new(),
            },
            Packet::Data {
                block: 65_535,
                data: vec![0xAB; 512],
            },
            Packet::Ack { block: 1 },
            Packet::Ack { block: 65_535 },
            Packet::Error {
                code: ErrorCode::FileNotFound,
                message: "File not found".into(),
            },
            Packet::Oack(vec![
                ("blksize".into(), "1024".into()),
                ("timeout".into(), "3".into()),
            ]),
        ];
        for packet in packets {
            let bytes = encode(&packet).expect("encode");
            let decoded = decode(&bytes, 65_464).expect("decode");
            assert_eq!(decoded, packet, "roundtrip failed for {packet:?}");
        }
    }

    #[test]
    fn request_without_options_roundtrips() {
        let bytes = encode(&Packet::Rrq(Request {
            filename: "a.txt".into(),
            mode: Mode::Octet,
            options: vec![],
        }))
        .expect("encode");
        assert_eq!(bytes, b"\x00\x01a.txt\x00octet\x00");
        assert_eq!(
            decode(&bytes, 512).expect("decode"),
            Packet::Rrq(Request {
                filename: "a.txt".into(),
                mode: Mode::Octet,
                options: vec![]
            })
        );
    }

    #[test]
    fn rejects_truncated_packets() {
        assert_eq!(decode(&[], 512), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0], 512), Err(DecodeError::Truncated));
        assert_eq!(
            decode(&[0, 4], 512),
            Err(DecodeError::Malformed("ACK must be exactly 4 bytes"))
        );
        assert_eq!(decode(&[0, 3, 0], 512), Err(DecodeError::Truncated));
    }

    #[test]
    fn rejects_unknown_opcode() {
        assert_eq!(decode(&[0, 99], 512), Err(DecodeError::UnknownOpcode(99)));
    }

    #[test]
    fn rejects_malformed_requests() {
        // Missing mode.
        assert!(decode(b"\x00\x01boot\x00", 512).is_err());
        // Empty filename.
        assert!(decode(b"\x00\x01\x00octet\x00", 512).is_err());
        // Non-terminated mode.
        assert!(decode(b"\x00\x01boot\x00octet", 512).is_err());
        // Unknown mode is distinguished from malformed requests.
        assert_eq!(
            decode(b"\x00\x01boot\x00binary\x00", 512),
            Err(DecodeError::UnsupportedMode("binary".into()))
        );
        // Non-UTF-8 filename.
        assert_eq!(
            decode(b"\x00\x01\xff\x00octet\x00", 512),
            Err(DecodeError::InvalidUtf8)
        );
    }

    #[test]
    fn rejects_oversized_requests_and_data() {
        let mut request = Vec::from(&b"\x00\x01"[..]);
        request.extend(std::iter::repeat_n(b'a', MAX_REQUEST_BYTES + 1));
        assert_eq!(decode(&request, 512), Err(DecodeError::TooLarge));

        let mut data = Vec::from(&b"\x00\x03\x00\x01"[..]);
        data.extend(std::iter::repeat_n(0u8, 513));
        assert_eq!(decode(&data, 512), Err(DecodeError::TooLarge));
        assert!(decode(&data, 513).is_ok());
    }

    #[test]
    fn rejects_bad_options() {
        // Empty option value.
        assert!(decode(b"\x00\x01f\x00octet\x00blksize\x00\x00", 512).is_err());
        // Unterminated option name.
        assert!(decode(b"\x00\x01f\x00octet\x00blksize", 512).is_err());
        // Non-ASCII option value.
        assert!(decode(b"\x00\x01f\x00octet\x00blksize\x00\xff\x00", 512).is_err());
    }

    #[test]
    fn error_message_sanitising() {
        assert_eq!(sanitize_text("a\nb\r\0c", 32), "a b  c");
        assert_eq!(sanitize_text("abcdef", 4), "abcd");
        // Never split a multi-byte character.
        assert_eq!(sanitize_text("ééé", 3), "é");
    }

    #[test]
    fn encoding_rejects_interior_nul() {
        let packet = Packet::Error {
            code: ErrorCode::Undefined,
            message: "bad\0msg".into(),
        };
        assert_eq!(encode(&packet), Err(EncodeError::InteriorNul));
    }

    #[test]
    fn error_packet_without_message_is_accepted() {
        assert_eq!(
            decode(&[0, 5, 0, 1], 512),
            Ok(Packet::Error {
                code: ErrorCode::FileNotFound,
                message: String::new()
            })
        );
    }

    #[test]
    fn unknown_error_code_maps_to_undefined() {
        assert_eq!(
            decode(&[0, 5, 0, 42, b'x', 0], 512),
            Ok(Packet::Error {
                code: ErrorCode::Undefined,
                message: "x".into()
            })
        );
    }

    #[test]
    fn oack_requires_at_least_one_option() {
        assert!(decode(&[0, 6], 512).is_err());
        assert!(decode(&[0, 6, 0], 512).is_err());
    }
}

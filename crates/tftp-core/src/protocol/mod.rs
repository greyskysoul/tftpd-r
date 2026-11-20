//! TFTP wire protocol: packet types, codec and RFC 2347/2348/2349 options.

pub mod codec;
pub mod options;
pub mod packet;

pub use codec::{decode, encode, encode_into, sanitize_text, MAX_REQUEST_BYTES};
pub use options::{negotiate, Negotiated, OptionError, MAX_BLOCK_SIZE, MIN_BLOCK_SIZE};
pub use packet::{Mode, Options, Packet, Request};

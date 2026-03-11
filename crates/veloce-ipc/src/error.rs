use thiserror::Error;

/// Errors that can occur during IPC frame encoding or decoding.
#[derive(Debug, Error)]
pub enum IpcError {
    #[error("bad frame magic: 0x{0:08X} (expected 0x56454C43)")]
    BadMagic(u32),

    #[error("protocol version mismatch: got {got}, expected {expected}")]
    VersionMismatch { got: u8, expected: u8 },

    #[error("unknown message type discriminant: 0x{0:02X}")]
    UnknownMessageType(u8),

    #[error("payload too large: {size} bytes (max {max})")]
    PayloadTooLarge { size: u32, max: u32 },

    #[error("encode failed: {0}")]
    EncodeFailed(String),

    #[error("decode failed: {0}")]
    DecodeFailed(String),
}

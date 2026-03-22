/*!
Binary codec for VeloceCore IPC frames.

Read path:  bytes → Header → validate → decode payload → Envelope
Write path: Envelope → encode payload → Header → bytes
*/

use bytes::{Buf, BufMut, BytesMut};
use crate::{MAGIC, VERSION, MAX_PAYLOAD, HEADER_LEN, IpcError};
use crate::message::{Envelope, Flags, MessageType};

// ── HEADER ────────────────────────────────────────────────────────────────────

/// 12-byte fixed frame header.
#[derive(Debug, Clone)]
pub struct FrameHeader {
    pub version:     u8,
    pub msg_type:    MessageType,
    pub flags:       Flags,
    pub payload_len: u32,
}

impl FrameHeader {
    /// Encode into `dst`.  Caller must ensure 12 bytes available.
    pub fn encode(&self, dst: &mut BytesMut) {
        dst.put_u32(MAGIC);
        dst.put_u8(self.version);
        dst.put_u8(self.msg_type as u8);
        dst.put_u16_le(self.flags.bits());
        dst.put_u32_le(self.payload_len);
    }

    /// Attempt to decode a header from `src`.
    /// Returns `None` if fewer than `HEADER_LEN` bytes are available.
    pub fn decode(src: &mut BytesMut) -> Result<Option<Self>, IpcError> {
        if src.len() < HEADER_LEN {
            return Ok(None);
        }

        let magic = u32::from_be_bytes(src[0..4].try_into().unwrap());
        if magic != MAGIC {
            return Err(IpcError::BadMagic(magic));
        }

        let version = src[4];
        if version != VERSION {
            return Err(IpcError::VersionMismatch { got: version, expected: VERSION });
        }

        let msg_type_byte = src[5];
        let msg_type = MessageType::try_from(msg_type_byte)
            .map_err(|b| IpcError::UnknownMessageType(b))?;

        let flags_bits = u16::from_le_bytes(src[6..8].try_into().unwrap());
        let flags = Flags::from_bits_truncate(flags_bits);

        let payload_len = u32::from_le_bytes(src[8..12].try_into().unwrap());
        if payload_len > MAX_PAYLOAD {
            return Err(IpcError::PayloadTooLarge { size: payload_len, max: MAX_PAYLOAD });
        }

        src.advance(HEADER_LEN);
        Ok(Some(Self { version, msg_type, flags, payload_len }))
    }
}

// ── CODEC ─────────────────────────────────────────────────────────────────────

pub struct Codec;

impl Codec {
    /// Encode an `Envelope` into a complete frame (header + payload).
    pub fn encode(envelope: &Envelope, flags: Flags) -> Result<BytesMut, IpcError> {
        let payload = bincode::serialize(envelope)
            .map_err(|e| IpcError::EncodeFailed(e.to_string()))?;

        if payload.len() as u32 > MAX_PAYLOAD {
            return Err(IpcError::PayloadTooLarge {
                size: payload.len() as u32,
                max: MAX_PAYLOAD,
            });
        }

        let mut buf = BytesMut::with_capacity(HEADER_LEN + payload.len());
        let header = FrameHeader {
            version:     VERSION,
            msg_type:    envelope.msg_type(),
            flags,
            payload_len: payload.len() as u32,
        };
        header.encode(&mut buf);
        buf.extend_from_slice(&payload);
        Ok(buf)
    }

    /// Try to decode a complete frame from `src`.
    ///
    /// Returns `Ok(None)` if more data is needed (keep buffering).
    /// Advances `src` past the consumed bytes on success.
    ///
    /// Delegates to [`decode_safe`] — always use this function, never call
    /// `FrameHeader::decode` directly, as it advances the buffer before the
    /// payload has been checked.
    pub fn decode(src: &mut BytesMut) -> Result<Option<(FrameHeader, Envelope)>, IpcError> {
        Self::decode_safe(src)
    }

    /// Safe two-phase decode: peek, then consume.
    ///
    /// This is the version you should call from async read loops.
    pub fn decode_safe(src: &mut BytesMut) -> Result<Option<(FrameHeader, Envelope)>, IpcError> {
        // Phase 1: peek at header without consuming
        if src.len() < HEADER_LEN {
            return Ok(None);
        }

        let magic = u32::from_be_bytes(src[0..4].try_into().unwrap());
        if magic != MAGIC {
            return Err(IpcError::BadMagic(magic));
        }
        let version = src[4];
        if version != VERSION {
            return Err(IpcError::VersionMismatch { got: version, expected: VERSION });
        }
        let msg_type_byte = src[5];
        let msg_type = MessageType::try_from(msg_type_byte)
            .map_err(|b| IpcError::UnknownMessageType(b))?;
        let flags_bits = u16::from_le_bytes(src[6..8].try_into().unwrap());
        let flags = Flags::from_bits_truncate(flags_bits);
        let payload_len = u32::from_le_bytes(src[8..12].try_into().unwrap());
        if payload_len > MAX_PAYLOAD {
            return Err(IpcError::PayloadTooLarge { size: payload_len, max: MAX_PAYLOAD });
        }

        let total = HEADER_LEN + payload_len as usize;
        if src.len() < total {
            // Not enough data yet — do not consume anything.
            return Ok(None);
        }

        // Phase 2: consume
        src.advance(HEADER_LEN);
        let payload = src.split_to(payload_len as usize);
        let envelope: Envelope = bincode::deserialize(&payload)
            .map_err(|e| IpcError::DecodeFailed(e.to_string()))?;

        let header = FrameHeader { version, msg_type, flags, payload_len };
        Ok(Some((header, envelope)))
    }
}

// ── TESTS ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Body, HandshakeMsg, Capability};
    use crate::Envelope;

    fn sample_envelope() -> Envelope {
        Envelope::new(Body::Handshake(HandshakeMsg {
            app_name:     "test-app".into(),
            sdk_version:  "0.1.0".into(),
            capabilities: vec![Capability::SpawnNodes, Capability::RegistryRead],
            psk_hash:     None,
        }))
    }

    #[test]
    fn roundtrip_handshake() {
        let env   = sample_envelope();
        let flags = Flags::EXPECTS_ACK;
        let mut buf = Codec::encode(&env, flags).unwrap();
        let (hdr, decoded) = Codec::decode_safe(&mut buf).unwrap().unwrap();
        assert_eq!(hdr.msg_type, MessageType::Handshake);
        assert_eq!(hdr.flags, flags);
        assert_eq!(decoded.correlation_id, env.correlation_id);
    }

    #[test]
    fn partial_frame_returns_none() {
        let env  = sample_envelope();
        let mut full = Codec::encode(&env, Flags::empty()).unwrap();
        // Truncate by removing last 5 bytes
        full.truncate(full.len() - 5);
        let result = Codec::decode_safe(&mut full).unwrap();
        assert!(result.is_none());
        // Buffer should be untouched (zero consumed)
        assert!(!full.is_empty());
    }

    #[test]
    fn bad_magic_is_error() {
        let mut buf = BytesMut::from(&[0xDE, 0xAD, 0xBE, 0xEF, 1, 0, 0, 0, 0, 0, 0, 0][..]);
        assert!(matches!(Codec::decode_safe(&mut buf), Err(IpcError::BadMagic(_))));
    }

    #[test]
    fn ping_pong_roundtrip() {
        let env = Envelope::new(Body::Ping);
        let mut buf = Codec::encode(&env, Flags::URGENT).unwrap();
        let (_hdr, decoded) = Codec::decode_safe(&mut buf).unwrap().unwrap();
        assert!(matches!(decoded.body, Body::Ping));
    }
}
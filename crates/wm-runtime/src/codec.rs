//! Schema / Codec separation (WM-0312).
//!
//! Schema is not Codec (docs/native-runtime.md "Transport / Protocol"):
//! a [`MessageSchema`] is the semantic registry — which message ids exist and
//! at what protocol version; a [`Codec`] is the mechanical encode/decode that
//! frames a `(id, payload)` pair on the wire. A transport deals only in bytes
//! and never learns what an id means.
//!
//! Wire message: `[u32 BE length][u32 BE message id][payload]` — the length
//! prefix and id are framing + schema bookkeeping, `payload` is opaque to the
//! codec.

use std::marker::PhantomData;

use crate::framing::{FrameDecoder, FrameEncoder, FramingError, LengthPrefixed};

/// Semantic registry of message ids and protocol version.
pub trait MessageSchema {
    /// Protocol version this schema speaks; a version mismatch must be
    /// rejected at handshake time, never silently downgraded.
    const VERSION: u32;

    /// Whether `id` is a message this schema knows how to encode/decode.
    fn is_known(id: u32) -> bool;
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum CodecError {
    #[error("unknown message id {0}")]
    UnknownId(u32),
    #[error("frame too short to carry a message id")]
    ShortFrame,
    #[error("framing error: {0}")]
    Framing(#[from] FramingError),
}

/// Encode/decode typed `(id, payload)` messages over length-prefixed framing.
#[derive(Debug, Clone)]
pub struct Codec<S: MessageSchema> {
    framing: LengthPrefixed,
    _schema: PhantomData<S>,
}

impl<S: MessageSchema> Codec<S> {
    pub fn new(max_frame_len: usize) -> Self {
        Self {
            framing: LengthPrefixed::new(max_frame_len),
            _schema: PhantomData,
        }
    }

    /// Encode one message into wire bytes. Rejects ids outside the schema.
    pub fn encode(&self, id: u32, payload: &[u8]) -> Result<Vec<u8>, CodecError> {
        if !S::is_known(id) {
            return Err(CodecError::UnknownId(id));
        }
        let mut frame = Vec::with_capacity(4 + payload.len());
        frame.extend_from_slice(&id.to_be_bytes());
        frame.extend_from_slice(payload);
        Ok(self.framing.encode_frame(&frame))
    }

    /// Feed stream bytes; return every complete `(id, payload)` message.
    /// Frames with unknown ids are rejected with [`CodecError::UnknownId`].
    pub fn decode(&mut self, data: &[u8]) -> Result<Vec<(u32, Vec<u8>)>, CodecError> {
        let mut out = Vec::new();
        for frame in self.framing.feed(data)? {
            if frame.len() < 4 {
                return Err(CodecError::ShortFrame);
            }
            let id = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]);
            if !S::is_known(id) {
                return Err(CodecError::UnknownId(id));
            }
            out.push((id, frame[4..].to_vec()));
        }
        Ok(out)
    }

    /// Payload of the last feed still buffered (partial frame), in bytes.
    pub fn buffered_len(&self) -> usize {
        self.framing.buffered_len()
    }
}

/// Concrete v1 test schema: a tiny ping/pong/login message set.
///
/// Real game schemas (KBEngine, etc.) plug in behind the same trait; this one
/// exists so the codec is testable without a live protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestMessage {
    Ping = 1,
    Pong = 2,
    Login = 3,
    Logout = 4,
}

impl TestMessage {
    pub fn id(self) -> u32 {
        self as u32
    }
}

pub struct TestSchema;

impl MessageSchema for TestSchema {
    const VERSION: u32 = 1;
    fn is_known(id: u32) -> bool {
        matches!(id, 1..=4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip() {
        let mut codec = Codec::<TestSchema>::new(1024);
        let wire = codec.encode(TestMessage::Login.id(), b"user123").unwrap();
        assert_eq!(codec.decode(&wire).unwrap(), vec![(3, b"user123".to_vec())]);
    }

    #[test]
    fn wire_layout_is_len_then_id_then_payload() {
        let codec = Codec::<TestSchema>::new(1024);
        let wire = codec.encode(TestMessage::Ping.id(), b"").unwrap();
        // len = 4 (id) + 0 (payload) = 4; id = 2.
        assert_eq!(wire, vec![0, 0, 0, 4, 0, 0, 0, 1]);
    }

    #[test]
    fn multiple_messages_reassemble_in_order() {
        let mut codec = Codec::<TestSchema>::new(1024);
        let mut wire = codec.encode(TestMessage::Ping.id(), b"a").unwrap();
        wire.extend(codec.encode(TestMessage::Pong.id(), b"b").unwrap());
        wire.extend(codec.encode(TestMessage::Logout.id(), b"c").unwrap());
        assert_eq!(
            codec.decode(&wire).unwrap(),
            vec![(1, b"a".to_vec()), (2, b"b".to_vec()), (4, b"c".to_vec())]
        );
    }

    #[test]
    fn unknown_id_rejected_on_encode() {
        let codec = Codec::<TestSchema>::new(1024);
        assert_eq!(codec.encode(99, b"x"), Err(CodecError::UnknownId(99)));
    }

    #[test]
    fn unknown_id_rejected_on_decode() {
        let mut codec = Codec::<TestSchema>::new(1024);
        let wire = codec.encode(TestMessage::Ping.id(), b"x").unwrap();
        // Rewrite the id to 99, keeping the frame length.
        let mut bad = wire.clone();
        bad[4] = 0;
        bad[5] = 0;
        bad[6] = 0;
        bad[7] = 99;
        assert_eq!(codec.decode(&bad), Err(CodecError::UnknownId(99)));
    }

    #[test]
    fn short_frame_rejected() {
        let mut codec = Codec::<TestSchema>::new(1024);
        // A complete frame carrying only 2 payload bytes (< id width).
        let wire = [0, 0, 0, 2, 0xAB, 0xCD];
        assert_eq!(codec.decode(&wire), Err(CodecError::ShortFrame));
    }

    #[test]
    fn partial_feed_buffers_and_buffered_len_reports() {
        let mut codec = Codec::<TestSchema>::new(1024);
        let wire = codec.encode(TestMessage::Ping.id(), b"hello").unwrap();
        assert!(codec.decode(&wire[..3]).unwrap().is_empty());
        assert_eq!(codec.buffered_len(), 3);
        let msgs = codec.decode(&wire[3..]).unwrap();
        assert_eq!(msgs, vec![(1, b"hello".to_vec())]);
        assert_eq!(codec.buffered_len(), 0);
    }

    #[test]
    fn schema_version_is_1() {
        assert_eq!(TestSchema::VERSION, 1);
    }
}

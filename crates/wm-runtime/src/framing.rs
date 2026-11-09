//! Framing (WM-0311): message boundaries over a byte stream.
//!
//! A byte stream has no message boundaries; framing imposes them. The v1
//! frame format is length-prefixed: `[u32 BE length][payload]`, so a frame is
//! `length` payload bytes preceded by its own big-endian length. This is the
//! standard choice for game/load protocols and is trivially self-delimiting.
//!
//! Framing is purely mechanical — it knows nothing about message ids or
//! business semantics (that is WM-0312's Schema/Codec layer).

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum FramingError {
    /// A length prefix claimed more bytes than the configured maximum.
    #[error("frame length {0} exceeds maximum {1}")]
    TooLarge(u64, usize),
    /// Length prefix arithmetic overflowed.
    #[error("length prefix overflow")]
    Overflow,
}

/// Turn one logical frame into bytes ready to send.
pub trait FrameEncoder {
    fn encode_frame(&self, payload: &[u8]) -> Vec<u8>;
}

/// Reassemble a byte stream into complete frames.
pub trait FrameDecoder {
    /// Feed raw stream bytes; return every complete frame that became
    /// available, in order. Partial data is buffered internally.
    fn feed(&mut self, data: &[u8]) -> Result<Vec<Vec<u8>>, FramingError>;
}

/// Length-prefixed framing with a configurable maximum frame size.
///
/// Stateful on the decode side: partial frames are buffered until their
/// declared length is satisfied.
#[derive(Debug, Clone)]
pub struct LengthPrefixed {
    max_frame_len: usize,
    buf: Vec<u8>,
}

impl LengthPrefixed {
    pub fn new(max_frame_len: usize) -> Self {
        assert!(
            max_frame_len >= 4,
            "max frame length must hold at least a length prefix"
        );
        Self {
            max_frame_len,
            buf: Vec::new(),
        }
    }

    /// Bytes currently buffered waiting for the rest of a partial frame.
    pub fn buffered_len(&self) -> usize {
        self.buf.len()
    }
}

const LEN_PREFIX: usize = 4;

impl FrameEncoder for LengthPrefixed {
    fn encode_frame(&self, payload: &[u8]) -> Vec<u8> {
        assert!(
            payload.len() <= self.max_frame_len,
            "payload of {} bytes exceeds max frame length {}",
            payload.len(),
            self.max_frame_len
        );
        let mut out = Vec::with_capacity(LEN_PREFIX + payload.len());
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(payload);
        out
    }
}

impl FrameDecoder for LengthPrefixed {
    fn feed(&mut self, data: &[u8]) -> Result<Vec<Vec<u8>>, FramingError> {
        self.buf.extend_from_slice(data);
        let mut frames = Vec::new();
        loop {
            if self.buf.len() < LEN_PREFIX {
                break;
            }
            let len =
                u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) as u64;
            if len > self.max_frame_len as u64 {
                return Err(FramingError::TooLarge(len, self.max_frame_len));
            }
            let total = (len as usize)
                .checked_add(LEN_PREFIX)
                .ok_or(FramingError::Overflow)?;
            if self.buf.len() < total {
                break; // wait for the rest of the frame
            }
            let payload: Vec<u8> = self.buf[LEN_PREFIX..total].to_vec();
            self.buf.drain(..total);
            frames.push(payload);
        }
        Ok(frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_prepends_be_length() {
        let framing = LengthPrefixed::new(1024);
        assert_eq!(
            framing.encode_frame(b"abc"),
            vec![0, 0, 0, 3, b'a', b'b', b'c']
        );
        assert_eq!(framing.encode_frame(b""), vec![0, 0, 0, 0]);
    }

    #[test]
    fn feed_reassembles_single_frame() {
        let mut framing = LengthPrefixed::new(1024);
        let frames = framing.feed(&[0, 0, 0, 3, b'a', b'b', b'c']).unwrap();
        assert_eq!(frames, vec![b"abc".to_vec()]);
    }

    #[test]
    fn partial_feed_buffers_until_complete() {
        let mut framing = LengthPrefixed::new(1024);
        assert!(framing.feed(&[0, 0, 0, 5]).unwrap().is_empty());
        assert!(framing.feed(b"h").unwrap().is_empty());
        assert_eq!(framing.feed(b"ello").unwrap(), vec![b"hello".to_vec()]);
    }

    #[test]
    fn multiple_frames_in_one_feed() {
        let mut framing = LengthPrefixed::new(1024);
        let mut wire = framing.encode_frame(b"one");
        wire.extend(framing.encode_frame(b"two"));
        wire.extend(framing.encode_frame(b"three"));
        let frames = framing.feed(&wire).unwrap();
        assert_eq!(
            frames,
            vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()]
        );
    }

    #[test]
    fn empty_frame_round_trips() {
        let mut framing = LengthPrefixed::new(1024);
        let wire = framing.encode_frame(b"");
        assert_eq!(framing.feed(&wire).unwrap(), vec![Vec::<u8>::new()]);
    }

    #[test]
    fn oversize_frame_is_rejected() {
        let mut framing = LengthPrefixed::new(16);
        let wire = [0, 0, 0, 20, 0, 0, 0, 0];
        assert_eq!(framing.feed(&wire), Err(FramingError::TooLarge(20, 16)));
    }

    #[test]
    #[should_panic(expected = "exceeds max frame length")]
    fn encode_rejects_oversize_payload() {
        LengthPrefixed::new(4).encode_frame(&[0u8; 5]);
    }

    #[test]
    fn encode_feed_round_trip() {
        let mut framing = LengthPrefixed::new(1 << 20);
        let payloads: Vec<Vec<u8>> = vec![
            b"ping".to_vec(),
            vec![0u8; 0],
            vec![0xAB; 1000],
            b"pong-pong-pong".to_vec(),
        ];
        let mut wire = Vec::new();
        for p in &payloads {
            wire.extend(framing.encode_frame(p));
        }
        let mut chunks = Vec::new();
        for chunk in wire.chunks(7) {
            chunks.extend(framing.feed(chunk).unwrap());
        }
        assert_eq!(chunks, payloads);
    }
}

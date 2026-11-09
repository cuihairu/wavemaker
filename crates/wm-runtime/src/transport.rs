//! Transport trait (WM-0309).
//!
//! A `Transport` is a poll-based byte stream: `send` and `recv` never block,
//! reporting `WouldBlock` when no progress is possible. This keeps the core
//! deterministic (a real-IO adapter is just a transport whose `recv`/`send`
//! poll a non-blocking socket); async wrappers live behind the same trait.
//!
//! Transport knows only bytes — it never interprets game business messages
//! (docs/native-runtime.md "Transport / Protocol": "Transport should not know
//! what game business messages mean").

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// No progress possible right now; retry later.
    #[error("operation would block")]
    WouldBlock,
    /// The peer closed the connection.
    #[error("transport closed")]
    Closed,
    /// Underlying IO failure.
    #[error("io error: {0}")]
    Io(String),
}

/// Poll-based byte-stream connection.
pub trait Transport {
    /// Write up to `buf.len()` bytes; returns the number accepted.
    ///
    /// - `Ok(n)` with `n > 0`: `n` bytes consumed.
    /// - `Ok(0)`: no progress possible now (equivalent to `WouldBlock`).
    /// - `Err(WouldBlock)`: buffer full, retry later.
    /// - `Err(Closed)`: connection is closed.
    fn send(&mut self, buf: &[u8]) -> Result<usize, TransportError>;

    /// Read into `buf`.
    ///
    /// - `Ok(0)`: no data available now (connection open).
    /// - `Ok(n > 0)`: `n` bytes read.
    /// - `Err(Closed)`: peer closed (no more data will ever arrive).
    fn recv(&mut self, buf: &mut [u8]) -> Result<usize, TransportError>;

    fn is_open(&self) -> bool;

    fn close(&mut self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir {
    AtoB,
    BtoA,
}

#[derive(Debug)]
struct Shared {
    a_to_b: VecDeque<u8>,
    b_to_a: VecDeque<u8>,
    a_open: bool,
    b_open: bool,
}

/// In-memory full-duplex [`Transport`] for tests and simulations.
///
/// A pair is created with [`MockTransport::pair`]: bytes sent on one side are
/// readable on the other, so framing/codec tests can drive a real duplex link
/// without sockets.
#[derive(Debug, Clone)]
pub struct MockTransport {
    shared: Rc<RefCell<Shared>>,
    dir: Dir,
}

impl MockTransport {
    pub fn pair() -> (Self, Self) {
        let shared = Rc::new(RefCell::new(Shared {
            a_to_b: VecDeque::new(),
            b_to_a: VecDeque::new(),
            a_open: true,
            b_open: true,
        }));
        (
            Self {
                shared: shared.clone(),
                dir: Dir::AtoB,
            },
            Self {
                shared,
                dir: Dir::BtoA,
            },
        )
    }

    /// Inject bytes to be read by the next `recv` on this side.
    pub fn inject(&mut self, data: &[u8]) {
        let mut shared = self.shared.borrow_mut();
        let target = match self.dir {
            Dir::AtoB => &mut shared.b_to_a,
            Dir::BtoA => &mut shared.a_to_b,
        };
        target.extend(data);
    }

    /// Bytes this side has sent and the peer has not yet read.
    pub fn pending_out(&self) -> Vec<u8> {
        let shared = self.shared.borrow();
        match self.dir {
            Dir::AtoB => shared.a_to_b.iter().copied().collect(),
            Dir::BtoA => shared.b_to_a.iter().copied().collect(),
        }
    }
}

impl Transport for MockTransport {
    fn send(&mut self, buf: &[u8]) -> Result<usize, TransportError> {
        if !self.is_open() {
            return Err(TransportError::Closed);
        }
        let mut shared = self.shared.borrow_mut();
        let target = match self.dir {
            Dir::AtoB => &mut shared.a_to_b,
            Dir::BtoA => &mut shared.b_to_a,
        };
        target.extend(buf);
        Ok(buf.len())
    }

    fn recv(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        let mut shared = self.shared.borrow_mut();
        let source = match self.dir {
            Dir::AtoB => &mut shared.b_to_a,
            Dir::BtoA => &mut shared.a_to_b,
        };
        if source.is_empty() {
            let open = match self.dir {
                Dir::AtoB => shared.b_open,
                Dir::BtoA => shared.a_open,
            };
            return if open {
                Ok(0)
            } else {
                Err(TransportError::Closed)
            };
        }
        let n = buf.len().min(source.len());
        for (slot, byte) in buf.iter_mut().zip(source.drain(..n)) {
            *slot = byte;
        }
        Ok(n)
    }

    fn is_open(&self) -> bool {
        match self.dir {
            Dir::AtoB => self.shared.borrow().a_open,
            Dir::BtoA => self.shared.borrow().b_open,
        }
    }

    fn close(&mut self) {
        // Full teardown: closing one side closes the whole connection, so the
        // peer's recv sees Closed and its send is refused too.
        let mut shared = self.shared.borrow_mut();
        shared.a_open = false;
        shared.b_open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_recv_round_trips_full_duplex() {
        let (mut a, mut b) = MockTransport::pair();
        assert_eq!(a.send(b"hello"), Ok(5));
        assert_eq!(a.pending_out(), b"hello");
        let mut buf = [0u8; 16];
        assert_eq!(b.recv(&mut buf), Ok(5));
        assert_eq!(&buf[..5], b"hello");
        // Reverse direction.
        assert_eq!(b.send(b"world"), Ok(5));
        let mut buf2 = [0u8; 16];
        assert_eq!(a.recv(&mut buf2), Ok(5));
        assert_eq!(&buf2[..5], b"world");
    }

    #[test]
    fn recv_reports_no_data_when_open() {
        let (mut a, _b) = MockTransport::pair();
        let mut buf = [0u8; 4];
        assert_eq!(a.recv(&mut buf), Ok(0));
    }

    #[test]
    fn close_propagates_to_peer_recv_and_blocked_send() {
        let (mut a, mut b) = MockTransport::pair();
        a.close();
        assert!(!a.is_open());
        let mut buf = [0u8; 4];
        assert_eq!(b.recv(&mut buf), Err(TransportError::Closed));
        assert_eq!(b.send(b"x"), Err(TransportError::Closed));
    }

    #[test]
    fn inject_makes_data_readable() {
        let (mut a, _b) = MockTransport::pair();
        a.inject(b"payload");
        let mut buf = [0u8; 8];
        assert_eq!(a.recv(&mut buf), Ok(7));
        assert_eq!(&buf[..7], b"payload");
    }

    #[test]
    fn partial_recv_respects_buf_len() {
        let (mut a, mut b) = MockTransport::pair();
        a.send(b"0123456789").unwrap();
        let mut buf = [0u8; 4];
        assert_eq!(b.recv(&mut buf), Ok(4));
        assert_eq!(&buf, b"0123");
        assert_eq!(b.recv(&mut buf), Ok(4));
        assert_eq!(&buf, b"4567");
        assert_eq!(b.recv(&mut buf), Ok(2));
        assert_eq!(&buf[..2], b"89");
    }
}

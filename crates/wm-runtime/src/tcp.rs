//! First TCP Transport (WM-0310).
//!
//! A std-only, non-blocking [`TcpStream`] adapter for the [`Transport`] trait.
//! No async runtime: `send`/`recv` poll the socket and report `WouldBlock`
//! (or `Ok(0)` on recv) when no progress is possible, so the deterministic
//! event loop drives it like any other transport. Connect is the only blocking
//! step; a [`TcpTransport::connect_timeout`] variant bounds it.
//!
//! The adapter is transport-only: it moves bytes and never interprets them.

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::Duration;

use crate::transport::{Transport, TransportError};

/// Non-blocking TCP connection adapter.
#[derive(Debug)]
pub struct TcpTransport {
    stream: TcpStream,
    open: bool,
}

impl TcpTransport {
    /// Resolve `addr` and connect (blocking), then switch to non-blocking IO.
    pub fn connect(addr: &str) -> Result<Self, TransportError> {
        let stream = TcpStream::connect(addr).map_err(TransportError::from_io)?;
        Self::from_stream(stream)
    }

    /// Connect to a resolved address bounded by `timeout`.
    pub fn connect_timeout(addr: SocketAddr, timeout: Duration) -> Result<Self, TransportError> {
        let stream = TcpStream::connect_timeout(&addr, timeout).map_err(TransportError::from_io)?;
        Self::from_stream(stream)
    }

    fn from_stream(stream: TcpStream) -> Result<Self, TransportError> {
        stream
            .set_nonblocking(true)
            .map_err(TransportError::from_io)?;
        Ok(Self { stream, open: true })
    }
}

impl Transport for TcpTransport {
    fn send(&mut self, buf: &[u8]) -> Result<usize, TransportError> {
        if !self.open {
            return Err(TransportError::Closed);
        }
        match self.stream.write(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Err(TransportError::WouldBlock),
            Err(e) => {
                self.open = false;
                Err(TransportError::from_io(e))
            }
        }
    }

    fn recv(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        if !self.open {
            return Err(TransportError::Closed);
        }
        // A zero-length read is a no-op, never EOF: `Read::read` may return
        // Ok(0) for an empty buffer (some platforms do), which the code below
        // would misread as a peer close and drop a live connection.
        if buf.is_empty() {
            return Ok(0);
        }
        match self.stream.read(buf) {
            Ok(0) => {
                // Peer closed (FIN): no more data will ever arrive.
                self.open = false;
                Err(TransportError::Closed)
            }
            Ok(n) => Ok(n),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(0),
            Err(e) => {
                self.open = false;
                Err(TransportError::from_io(e))
            }
        }
    }

    fn is_open(&self) -> bool {
        self.open
    }

    fn close(&mut self) {
        self.open = false;
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Instant;

    /// Spin up a one-shot echo server on loopback; returns its address.
    fn start_echo_server() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local addr");
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                loop {
                    match stream.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            // Echo back immediately (may be partial; loopback
                            // accepts small writes whole).
                            let _ = stream.write_all(&buf[..n]);
                        }
                        Err(_) => break,
                    }
                }
            }
        });
        addr
    }

    /// Poll recv until `len` bytes arrive or the deadline passes.
    fn recv_with_retry(t: &mut TcpTransport, len: usize) -> Vec<u8> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut out = Vec::new();
        while out.len() < len && Instant::now() < deadline {
            let mut buf = [0u8; 256];
            match t.recv(&mut buf) {
                Ok(0) => thread::sleep(Duration::from_millis(5)),
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(TransportError::Closed) => break,
                Err(_) => thread::sleep(Duration::from_millis(5)),
            }
        }
        out
    }

    #[test]
    fn connects_and_echoes_over_loopback() {
        let addr = start_echo_server();
        let mut t = TcpTransport::connect_timeout(addr, Duration::from_secs(2)).unwrap();
        assert!(t.is_open());
        assert_eq!(t.send(b"ping"), Ok(4));
        let echoed = recv_with_retry(&mut t, 4);
        assert_eq!(echoed, b"ping");
    }

    #[test]
    fn recv_with_no_data_reports_zero_when_open() {
        let addr = start_echo_server();
        let mut t = TcpTransport::connect_timeout(addr, Duration::from_secs(2)).unwrap();
        let mut buf = [0u8; 8];
        // The peer has sent nothing; recv must not block and must not error.
        assert_eq!(t.recv(&mut buf), Ok(0));
    }

    #[test]
    fn recv_with_empty_buffer_does_not_close_a_live_connection() {
        let addr = start_echo_server();
        let mut t = TcpTransport::connect_timeout(addr, Duration::from_secs(2)).unwrap();
        assert_eq!(t.send(b"ping"), Ok(4));
        // A zero-length recv returns Ok(0) without touching the socket; it is
        // not a peer close and must not drop the connection.
        let mut empty: [u8; 0] = [];
        assert_eq!(
            t.recv(&mut empty),
            Ok(0),
            "empty buffer read is not a close"
        );
        assert!(t.is_open(), "connection must survive an empty-buffer recv");
        let echoed = recv_with_retry(&mut t, 4);
        assert_eq!(echoed, b"ping", "echo still works after the empty read");
    }

    #[test]
    fn close_makes_send_and_recv_refused() {
        let addr = start_echo_server();
        let mut t = TcpTransport::connect_timeout(addr, Duration::from_secs(2)).unwrap();
        t.close();
        assert!(!t.is_open());
        assert_eq!(t.send(b"x"), Err(TransportError::Closed));
        let mut buf = [0u8; 4];
        assert_eq!(t.recv(&mut buf), Err(TransportError::Closed));
    }

    #[test]
    fn connect_to_unreachable_port_is_an_error() {
        // Bind then drop to get a definitely-closed port, then try to connect.
        let addr = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        };
        assert!(TcpTransport::connect_timeout(addr, Duration::from_millis(200)).is_err());
    }
}

//! Bot process fault injection (WM-0218): a bots process dying mid-session
//! must surface as a prompt, bounded EOF on the Supervisor side — never a
//! hang. The session is a REAL Bridge conversation (the conformance
//! harness = production Bootstrap over a Unix socket with the runtime's
//! own framing), and the fault is a real SIGKILL of the child, the way an
//! OOM kill or an operator `pkill` ends a bots process.
//!
//! Requires `python3` on PATH (the bridge IS Python).

#![cfg(unix)]

use std::io::Read;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use wm_runtime::framing::{FrameDecoder, LengthPrefixed};

const READ_TIMEOUT: Duration = Duration::from_secs(5);

fn harness_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../engine-adapters/kbengine/conformance/harness.py")
}

/// Read frames until one is complete; socket EOF (Ok(0) on read) surfaces
/// as Err — exactly the signal the Supervisor reacts to.
fn read_first_frame(
    stream: &mut UnixStream,
    decoder: &mut LengthPrefixed,
) -> Result<Vec<u8>, ()> {
    let mut buf = [0u8; 8192];
    loop {
        let n = stream.read(&mut buf).map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        for frame in decoder.feed(&buf[..n]).unwrap() {
            return Ok(frame);
        }
    }
}

#[test]
fn bots_process_death_surfaces_as_eof_not_hang() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("bridge.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let mut child = Command::new("python3")
        .arg(harness_path())
        .arg(&socket_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("python3 must be on PATH");

    let (mut stream, _) = listener.accept().unwrap();
    stream.set_read_timeout(Some(READ_TIMEOUT)).unwrap();
    let mut decoder = LengthPrefixed::new(1024 * 1024);

    // The session is live: the bridge's handshake arrives.
    let handshake = read_first_frame(&mut stream, &mut decoder).expect("handshake frame");
    let handshake: serde_json::Value = serde_json::from_slice(&handshake).unwrap();
    assert_eq!(handshake["kind"], "handshake");

    // The fault: SIGKILL the bots process mid-session (OOM / operator).
    child.kill().expect("kill the bots process");

    // The death must be visible on the socket promptly: EOF well inside
    // the read timeout, and never a hang.
    let started = Instant::now();
    loop {
        match read_first_frame(&mut stream, &mut decoder) {
            Err(()) => break,
            Ok(_) => assert!(
                started.elapsed() < Duration::from_secs(2),
                "socket kept delivering after process death"
            ),
        }
        assert!(
            started.elapsed() < READ_TIMEOUT,
            "EOF did not arrive after process death"
        );
    }

    // The child is reaped as signal-killed — the Supervisor's exit record
    // for this fault.
    let status = child.wait().expect("reap the bots process");
    assert!(
        std::os::unix::process::ExitStatusExt::signal(&status).is_some(),
        "child was not killed by a signal: {status:?}"
    );
}

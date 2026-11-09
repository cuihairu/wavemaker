//! KBEngine adapter protocol conformance (WM-0216, docs/testing.md
//! "Contract / Conformance: Rust Supervisor <-> Python Bridge").
//!
//! Spawns the real Python bridge (engine-adapters/kbengine/conformance/
//! harness.py — the production Bootstrap, no protocol-side test doubles)
//! and plays the Supervisor over a real Unix socket with the SAME
//! length-prefixed framing the runtime uses (wm_runtime::framing). The
//! conversation walks the docs/bridge.md protocol end to end:
//!
//!   handshake -> ack -> ApplyPlan -> Start -> event batch ->
//!   SubscribeEvents replay (identical event_id, dedup-stable) ->
//!   StopAdmission (idempotent replay) -> heartbeat shape ->
//!   Drain -> Shutdown -> session EOF.
//!
//! Requires `python3` on PATH (the bridge IS Python; that is the point of
//! a cross-language conformance test).

#![cfg(unix)]

use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use wm_runtime::framing::{FrameDecoder, FrameEncoder, LengthPrefixed};

const READ_TIMEOUT: Duration = Duration::from_secs(10);
const PUMP_DELAY_S: f64 = 0.1;

struct Wire {
    stream: UnixStream,
    encoder: LengthPrefixed,
    decoder: LengthPrefixed,
    /// Frames decoded but not yet consumed. One `read` burst can deliver
    /// several frames (the pump answers a command and sends events on the
    /// same tick); buffering here keeps a match-and-return from dropping
    /// the frames that shared the burst.
    pending: Vec<Value>,
}

impl Wire {
    fn new(stream: UnixStream) -> Self {
        stream.set_read_timeout(Some(READ_TIMEOUT)).unwrap();
        Self {
            stream,
            encoder: LengthPrefixed::new(1024 * 1024),
            decoder: LengthPrefixed::new(1024 * 1024),
            pending: Vec::new(),
        }
    }

    fn send(&mut self, message: &Value) {
        let payload = serde_json::to_vec(message).unwrap();
        let frame = self.encoder.encode_frame(&payload);
        self.stream.write_all(&frame).unwrap();
        self.stream.flush().unwrap();
    }

    /// Next frame whose `kind` is one of `wanted`; transport noise
    /// (heartbeats, unrelated batches) is skipped, as a real Supervisor
    /// consumer would.
    fn next_message(&mut self, wanted: &[&str]) -> Value {
        let mut buf = [0u8; 8192];
        loop {
            if let Some(index) = self.pending.iter().position(|m| {
                m.get("kind")
                    .and_then(|k| k.as_str())
                    .is_some_and(|k| wanted.contains(&k))
            }) {
                return self.pending.remove(index);
            }
            let n = self.stream.read(&mut buf).expect("socket read");
            if n == 0 {
                panic!("EOF while waiting for {:?}", wanted);
            }
            for frame in self.decoder.feed(&buf[..n]).unwrap() {
                let message: Value = serde_json::from_slice(&frame)
                    .expect("frame is valid JSON");
                self.pending.push(message);
            }
        }
    }

    /// Read until the peer closes the session (post-Shutdown teardown).
    fn wait_for_eof(&mut self) {
        let deadline = Instant::now() + READ_TIMEOUT;
        self.stream
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let mut buf = [0u8; 4096];
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => return,
                Ok(_) => {}
                Err(e)
                    if e.kind() == ErrorKind::WouldBlock
                        || e.kind() == ErrorKind::TimedOut => {}
                Err(e) => panic!("unexpected read error: {e}"),
            }
            assert!(
                Instant::now() < deadline,
                "session never closed after shutdown"
            );
        }
    }
}

fn command(operation_id: &str, name: &str, payload: Value) -> Value {
    json!({
        "kind": "command",
        "operation_id": operation_id,
        "generation": 7,
        "command_name": name,
        "payload": payload,
    })
}

fn expect_ok(wire: &mut Wire, operation_id: &str) -> Value {
    let result = wire.next_message(&["command_result"]);
    assert_eq!(
        result["operation_id"].as_str().unwrap_or(""),
        operation_id,
        "result for the wrong operation"
    );
    assert_eq!(result["generation"].as_i64().unwrap_or(-1), 7);
    assert_eq!(result["result"]["code"], "OK", "unexpected result");
    result
}

fn harness_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../engine-adapters/kbengine/conformance/harness.py")
}

#[test]
fn bridge_conformance_over_unix_socket() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("bridge.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let mut child = Command::new("python3")
        .arg(harness_path())
        .arg(&socket_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("python3 must be on PATH to run the bridge conformance");

    let (stream, _) = listener.accept().unwrap();
    let mut wire = Wire::new(stream);

    // 1. Handshake: the honest capability report (docs/bridge.md —
    // declarations verified by conformance, not a hand-written file).
    let handshake = wire.next_message(&["handshake"]);
    assert_eq!(handshake["kind"], "handshake");
    assert_eq!(handshake["protocol_version"], 1);
    assert_eq!(handshake["engine_name"], "kbengine");
    assert_eq!(handshake["backend_id"], "kb-conf");
    assert_eq!(handshake["backend_generation"], 12);
    assert_eq!(handshake["capabilities"]["process"]["start"], "supported");
    assert_eq!(handshake["capabilities"]["bots"]["create_batch"], "supported");
    assert_eq!(
        handshake["capabilities"]["observability"]["login_success"],
        "unknown"
    );
    assert_eq!(handshake["scheduling_resolution_ms"], 100);
    assert_eq!(handshake["arrival_batch_ms"], 1000);

    // 2. Ack: session accepted, generation assigned.
    wire.send(&json!({
        "kind": "handshake_ack",
        "generation": 7,
        "result": {"code": "OK", "reason_code": "", "detail": ""},
    }));

    // 3. ApplyPlan: the frozen ShardPlan with shard attribution.
    wire.send(&command(
        "conf-apply",
        "apply_plan",
        json!({
            "swell_id": "sw-conf",
            "plan_digest": "sha256:conf",
            "shard_id": "shard-conf",
            "batches": [{"offset_ms": 0, "bot_count": 1}],
        }),
    ));
    expect_ok(&mut wire, "conf-apply");

    // 4. Start: the bridge fires its schedule; the engine surfaces the
    // created bot on a later tick (S8), producing a connection event.
    wire.send(&command("conf-start", "start", json!({})));
    expect_ok(&mut wire, "conf-start");
    let batch = wire.next_message(&["events"]);
    assert_eq!(batch["kind"], "events");
    assert_eq!(batch["backend_id"], "kb-conf");
    assert_eq!(batch["cursor"], 1);
    let event = &batch["events"][0];
    for field in [
        "event_id",
        "backend_id",
        "swell_id",
        "shard_id",
        "bot_id",
        "sequence",
        "source_timestamp",
        "observed_timestamp",
        "event_type",
        "payload",
        "schema_version",
    ] {
        assert!(
            event.get(field).is_some(),
            "event envelope lacks {field}"
        );
    }
    assert_eq!(event["event_id"], "kb-conf:1");
    assert_eq!(event["sequence"], 1);
    assert_eq!(event["swell_id"], "sw-conf");
    assert_eq!(event["shard_id"], "shard-conf");
    assert_eq!(event["event_type"], "BotCreated");
    assert_eq!(event["bot_id"], 4101);
    assert_eq!(event["schema_version"], 1);

    // 5. SubscribeEvents: cursor resume replays the SAME envelope —
    // event_id-stable so Supervisor-side dedup collapses the overlap.
    wire.send(&command(
        "conf-sub",
        "subscribe_events",
        json!({"cursor": 0}),
    ));
    expect_ok(&mut wire, "conf-sub");
    let replay = wire.next_message(&["events"]);
    assert_eq!(replay["events"][0], *event, "replay is dedup-stable");
    assert_eq!(replay["cursor"], 1);
    assert!(replay.get("gap").is_none());

    // 6. StopAdmission: idempotent without a key — a different
    // operation_id still returns OK.
    wire.send(&command("conf-stop-1", "stop_admission", json!({})));
    expect_ok(&mut wire, "conf-stop-1");
    wire.send(&command("conf-stop-2", "stop_admission", json!({})));
    expect_ok(&mut wire, "conf-stop-2");

    // 7. Heartbeat: liveness plus transport, process, and bridge metrics.
    let beat = wire.next_message(&["heartbeat"]);
    assert_eq!(beat["kind"], "heartbeat");
    assert!(beat["sequence"].as_i64().unwrap_or(0) >= 1);
    assert_eq!(beat["generation"], 7);
    assert!(beat["bridge_state"].is_string());
    assert!(beat["ipc"].is_object());
    for field in ["cpu_seconds", "rss_bytes", "fd_count"] {
        assert!(
            beat["process"].get(field).is_some(),
            "heartbeat process metrics lack {field}"
        );
    }
    for field in
        ["queue_depth", "queue_dropped", "callback_lag_ms", "callback_lag_max_ms"]
    {
        assert!(
            beat["metrics"].get(field).is_some(),
            "heartbeat bridge metrics lack {field}"
        );
    }
    assert!(beat["monotonic"].is_number());

    // 8. Drain: one-way, session stays live.
    wire.send(&command("conf-drain", "drain", json!({})));
    expect_ok(&mut wire, "conf-drain");

    // 9. Shutdown: the ack goes out, then the bridge tears the session
    // down and the process exits cleanly (0) — termination is the
    // Supervisor's job, not the script's.
    wire.send(&command("conf-shutdown", "shutdown", json!({})));
    expect_ok(&mut wire, "conf-shutdown");
    wire.wait_for_eof();
    let output = child.wait_with_output().expect("harness process");
    assert!(
        output.status.success(),
        "harness exited abnormally; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

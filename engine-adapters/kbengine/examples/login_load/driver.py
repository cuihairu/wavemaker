#!/usr/bin/env python3
"""End-to-end login load test driver (WM-0217 sample, Supervisor side).

Plays the wavemaker Supervisor against a real KBEngine bots process running
the Bridge (see run_e2e.sh). Drives one swell over the same wire protocol
the Rust Supervisor speaks ([u32 BE length][UTF-8 JSON]):

    handshake -> ack -> ApplyPlan -> Start -> SubscribeEvents ->
    watch login events -> StopAdmission -> Drain -> Shutdown

and prints the observed connection events plus heartbeat metrics as the
swell runs, with a final summary.

Usage:
    python3 driver.py <socket_path> <plan.json> [settle_seconds]

Prerequisite: the bots process must be started with WAVEMAKER_BRIDGE_IPC
pointing at <socket_path> (run_e2e.sh does this).
"""

import json
import socket
import struct
import sys
import time
from collections import Counter, deque

DEFAULT_SETTLE_S = 4.0


def recv_exact(conn, count):
    buf = b""
    while len(buf) < count:
        chunk = conn.recv(count - len(buf))
        if not chunk:
            raise EOFError("session closed")
        buf += chunk
    return buf


def recv_frame(conn):
    (length,) = struct.unpack(">I", recv_exact(conn, 4))
    return json.loads(recv_exact(conn, length))


def send_frame(conn, message):
    payload = json.dumps(message).encode()
    conn.sendall(struct.pack(">I", len(payload)) + payload)


class Driver:
    """Frame-level Supervisor half. Decoded frames are buffered so a read
    that delivers several messages never loses the ones after the target."""

    def __init__(self, conn):
        self.conn = conn
        self.pending = deque()

    def next_message(self, wanted):
        while True:
            while self.pending:
                message = self.pending.popleft()
                if message.get("kind") in wanted:
                    return message
            self.pending.append(recv_frame(self.conn))

    def command(self, operation_id, name, payload, generation=1):
        send_frame(self.conn, {
            "kind": "command",
            "operation_id": operation_id,
            "generation": generation,
            "command_name": name,
            "payload": payload,
        })

    def expect_ok(self, operation_id):
        result = self.next_message(["command_result"])
        assert result.get("operation_id") == operation_id, result
        assert result["result"]["code"] == "OK", result
        print("  ok  %-14s %s" % (operation_id, result["result"]["code"]))


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    socket_path, plan_path = sys.argv[1], sys.argv[2]
    settle_s = float(sys.argv[3]) if len(sys.argv) > 3 else DEFAULT_SETTLE_S
    with open(plan_path) as handle:
        plan = json.load(handle)
    total_bots = sum(batch["bot_count"] for batch in plan["batches"])

    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(socket_path)
    server.listen(1)
    print("waiting for the bridge at %s ..." % socket_path)
    conn, _ = server.accept()
    conn.settimeout(30.0)
    driver = Driver(conn)

    handshake = driver.next_message(["handshake"])
    print("handshake: engine=%s backend=%s generation=%s" % (
        handshake["engine_name"], handshake["backend_id"],
        handshake["backend_generation"]))
    send_frame(conn, {
        "kind": "handshake_ack",
        "generation": 1,
        "result": {"code": "OK", "reason_code": "", "detail": ""},
    })

    driver.command("sample-apply", "apply_plan", plan)
    driver.expect_ok("sample-apply")
    driver.command("sample-start", "start", {})
    driver.expect_ok("sample-start")
    driver.command("sample-sub", "subscribe_events", {"cursor": 0})
    driver.expect_ok("sample-sub")

    # Watch the swell: connection events are the login progress signal
    # (BotCreated when a bot surfaces in the engine, BotDisconnected when
    # it leaves). Heartbeats carry bridge/process metrics; a sample
    # prints them so the load behaviour is visible.
    counts = Counter()
    queue_depth = 0
    last_event_time = time.monotonic()
    last_beat = None
    while True:
        message = driver.next_message(["events", "heartbeat"])
        if message["kind"] == "events":
            for event in message.get("events", []):
                counts[event["event_type"]] += 1
                print("  event %-16s bot_id=%s" % (
                    event["event_type"], event["bot_id"]))
            last_event_time = time.monotonic()
        else:
            last_beat = message
            metrics = message.get("metrics", {})
            queue_depth = metrics.get("queue_depth", queue_depth)
        quiet = time.monotonic() - last_event_time
        created = counts.get("BotCreated", 0)
        if created >= total_bots and quiet >= settle_s:
            break

    driver.command("sample-stop", "stop_admission", {})
    driver.expect_ok("sample-stop")
    driver.command("sample-drain", "drain", {})
    driver.expect_ok("sample-drain")
    driver.command("sample-shutdown", "shutdown", {})
    driver.expect_ok("sample-shutdown")

    print("\nsummary: %s" % dict(counts))
    if last_beat is not None:
        print("last heartbeat: bridge_state=%s process=%s metrics=%s" % (
            last_beat.get("bridge_state"), last_beat.get("process"),
            last_beat.get("metrics")))
    ok = counts.get("BotCreated", 0) >= total_bots
    print("result: %s (%d/%d bots surfaced)" % (
        "PASS" if ok else "FAIL", counts.get("BotCreated", 0), total_bots))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())

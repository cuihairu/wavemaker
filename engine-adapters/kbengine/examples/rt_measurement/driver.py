#!/usr/bin/env python3
"""Supervisor-side driver for the RT measurement harness (WM-0010).

Plays the Supervisor over the Bridge protocol: acknowledges the session,
applies a small login swell (so RT numbers include real engine activity),
keeps a command flowing through the Bridge queue, samples heartbeats, then
shuts the session down. Prints the observed callback-lag series summary.

Usage:  python3 driver.py <socket_path> [seconds]
"""

import json
import socket
import struct
import sys
import time
from collections import Counter, deque

MEASURE_SECONDS = 15.0
COMMANDS_PER_SECOND = 20.0


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


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    socket_path = sys.argv[1]
    measure_s = float(sys.argv[2]) if len(sys.argv) > 2 else MEASURE_SECONDS

    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(socket_path)
    server.listen(1)
    conn, _ = server.accept()
    conn.settimeout(30.0)
    pending = deque()

    def next_message(wanted):
        while True:
            while pending:
                message = pending.popleft()
                if message.get("kind") in wanted:
                    return message
            pending.append(recv_frame(conn))

    def command(operation_id, payload):
        send_frame(conn, {
            "kind": "command",
            "operation_id": operation_id,
            "generation": 1,
            "command_name": "stop_admission",
            "payload": payload,
        })

    handshake = next_message(["handshake"])
    print("handshake: backend=%s" % handshake["backend_id"])
    send_frame(conn, {
        "kind": "handshake_ack",
        "generation": 1,
        "result": {"code": "OK", "reason_code": "", "detail": ""},
    })
    send_frame(conn, {
        "kind": "command",
        "operation_id": "rt-apply",
        "generation": 1,
        "command_name": "apply_plan",
        "payload": {
            "swell_id": "sw-rt",
            "plan_digest": "sha256:rt",
            "shard_id": "shard-rt",
            "batches": [{"offset_ms": 0, "bot_count": 3}],
        },
    })
    next_message(["command_result"])
    send_frame(conn, {
        "kind": "command",
        "operation_id": "rt-start",
        "generation": 1,
        "command_name": "start",
        "payload": {},
    })
    next_message(["command_result"])
    print("swell running; measuring for %.0fs" % measure_s)

    # Keep the Bridge queue fed while the login swell works the engine.
    lag_samples = []
    depth_samples = []
    results = Counter()
    started = time.monotonic()
    next_command = started
    command_no = 0
    while time.monotonic() - started < measure_s:
        now = time.monotonic()
        if now >= next_command:
            command_no += 1
            command("rt-cmd-%d" % command_no, {})
            next_command = now + 1.0 / COMMANDS_PER_SECOND
        message = next_message(["heartbeat", "command_result", "events"])
        if message["kind"] == "heartbeat":
            metrics = message.get("metrics", {})
            lag_samples.append(metrics.get("callback_lag_ms"))
            depth_samples.append(metrics.get("queue_depth"))
        elif message["kind"] == "command_result":
            results[message["result"]["code"]] += 1

    send_frame(conn, {
        "kind": "command",
        "operation_id": "rt-stop",
        "generation": 1,
        "command_name": "stop_admission",
        "payload": {},
    })
    next_message(["command_result"])
    send_frame(conn, {
        "kind": "command",
        "operation_id": "rt-shutdown",
        "generation": 1,
        "command_name": "shutdown",
        "payload": {},
    })
    next_message(["command_result"])

    measured_lag = [x for x in lag_samples if x is not None]
    measured_lag.sort()
    print("commands: %s" % dict(results))
    print("heartbeat samples: %d" % len(measured_lag))
    if measured_lag:
        print("callback_lag_ms: median=%.3f p95=%.3f max=%.3f" % (
            measured_lag[len(measured_lag) // 2],
            measured_lag[int(len(measured_lag) * 0.95)],
            measured_lag[-1]))
    return 0


if __name__ == "__main__":
    sys.exit(main())

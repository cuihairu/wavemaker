"""Unit tests for the WM-0202 Bridge local IPC client.

Run from the adapter root:  cd engine-adapters/kbengine && python3 -m pytest tests
Every socket test binds a real loopback listener (AF_UNIX under /tmp, or
127.0.0.1 TCP) in-process: no Supervisor is needed. Runtime conformance
against a real Supervisor is WM-0216.
"""

import json
import os
import socket
import struct
import tempfile
import threading

from bridge.bootstrap import Bootstrap, BootstrapConfig
from bridge.capability import CAPABILITIES, PROTOCOL_VERSION
from bridge.command_queue import CommandQueue
from bridge.commands import (
    RESULT_OK,
    RESULT_UNSUPPORTED,
    build_command_result,
    parse_command,
)
from bridge.engine_port import EnginePort, FakeEngine
from bridge.handshake import HANDSHAKE_ACK_KIND, build_handshake, parse_handshake_ack
from bridge.ipc_client import (
    DEFAULT_OUTBOUND_LIMIT,
    IpcAddressError,
    IpcClient,
    OutboundQueue,
    encode_frame,
    parse_address,
)


def make_bootstrap(**cfg):
    """Same construction as test_bootstrap.make_bootstrap (duplicated here
    rather than imported across test modules, which breaks under direct
    file execution): (fake_engine, bootstrap) via the production port."""
    config = BootstrapConfig(
        ipc_address=cfg.get("ipc", ""),
        token=cfg.get("token", ""),
        swell_id=cfg.get("swell", ""),
        backend_id=cfg.get("backend", ""),
        backend_generation=cfg.get("generation", 0),
    )
    fe = FakeEngine()
    return fe, Bootstrap(
        EnginePort(fe), config,
        reconnect_initial_s=cfg.get("reconnect_initial", 1.0),
        reconnect_max_s=cfg.get("reconnect_max", 10.0),
    )


def frame(payload):
    """Manual frame builder used to craft raw/garbage traffic."""
    return struct.pack(">I", len(payload)) + payload


class LoopbackServer(object):
    """One-shot local listener: accepts exactly one connection and records
    what the client sent, with helpers to push frames back."""

    def __init__(self, family, address):
        self._family = family
        self._address = address
        self._sock = socket.socket(family, socket.SOCK_STREAM)
        self._sock.bind(address)
        self._port = (
            self._sock.getsockname()[1] if family == socket.AF_INET else 0
        )
        self._sock.listen(1)
        self._conn = None
        self._buf = bytearray()
        self._accept_thread = threading.Thread(target=self._accept)
        self._accept_thread.daemon = True
        self._accept_thread.start()

    @classmethod
    def unix(cls, path=None):
        if path is None:
            path = os.path.join(
                tempfile.mkdtemp(prefix="wm-ipc-test-"), "bridge.sock"
            )
        return cls(socket.AF_UNIX, path)
    @classmethod
    def tcp(cls):
        # Bind once, with port 0 letting the kernel pick a free port —
        # binding a second socket to a port still held by a probe socket
        # would fail with EADDRINUSE.
        return cls(socket.AF_INET, ("127.0.0.1", 0))

    @property
    def address(self):
        if self._family == socket.AF_UNIX:
            return self._address
        return "tcp:127.0.0.1:%d" % self._port

    def _accept(self):
        try:
            self._conn, _ = self._sock.accept()
            self._conn.settimeout(5.0)
        except OSError:
            self._conn = None

    def wait_connected(self, timeout=5.0):
        self._accept_thread.join(timeout)
        assert self._conn is not None, "client never connected"
        return self._conn

    def recv_frame(self, timeout=5.0):
        """Receive exactly one complete frame from the client."""
        self.wait_connected(timeout)
        self._conn.settimeout(timeout)
        while True:
            if len(self._buf) >= 4:
                (length,) = struct.unpack_from(">I", self._buf, 0)
                if len(self._buf) >= 4 + length:
                    payload = bytes(self._buf[4:4 + length])
                    del self._buf[:4 + length]
                    return json.loads(payload.decode("utf-8"))
            chunk = self._conn.recv(65536)
            assert chunk != b"", "client closed the connection"
            self._buf.extend(chunk)

    def send_message(self, message):
        self.wait_connected()
        self._conn.sendall(encode_frame(message))

    def send_raw(self, data):
        self.wait_connected()
        self._conn.sendall(data)

    def close(self):
        # The client's connect() returns once the kernel completes the
        # handshake, which can happen before the accept thread has stored
        # the connection: join it first, then close whatever exists, or a
        # peer-disconnect test waits on a FIN that never arrives.
        self._accept_thread.join(2.0)
        for sock in (self._conn, self._sock):
            if sock is not None:
                try:
                    sock.close()
                except OSError:
                    pass


# -- address parsing -------------------------------------------------------


def test_parse_address_forms():
    assert parse_address("/tmp/wm.sock") == ("unix", "/tmp/wm.sock")
    assert parse_address("unix:/tmp/wm.sock") == ("unix", "/tmp/wm.sock")
    assert parse_address("tcp:127.0.0.1:7000") == ("tcp", "127.0.0.1", 7000)
    assert parse_address("tcp:localhost:7000") == ("tcp", "localhost", 7000)


def test_parse_address_rejects_garbage():
    for bad in ("", "  ", "sock.sock", "unix:", "tcp:", "tcp:host",
                "tcp:host:notaport"):
        try:
            parse_address(bad)
        except IpcAddressError:
            pass
        else:
            raise AssertionError("accepted bad address %r" % bad)


# -- framing ---------------------------------------------------------------


def test_encode_frame_layout():
    raw = encode_frame({"a": 1})
    (length,) = struct.unpack_from(">I", raw, 0)
    assert raw[4:] == b'{"a":1}'
    assert length == len(raw) - 4


def test_frame_reader_reassembles_split_and_coalesced_frames():
    from bridge.ipc_client import FrameReader

    reader = FrameReader()
    frames = encode_frame({"n": 1}) + encode_frame({"n": 2})
    # Half frames first.
    assert reader.feed(frames[:3]) == []
    assert reader.feed(frames[3:9]) == []
    # The remainder carries both frames at once.
    assert reader.feed(frames[9:]) == [{"n": 1}, {"n": 2}]


def test_frame_reader_rejects_bad_lengths_and_payloads():
    from bridge.ipc_client import FrameError, FrameReader

    reader = FrameReader()
    try:
        reader.feed(struct.pack(">I", 0))
    except FrameError:
        pass
    else:
        raise AssertionError("zero-length frame accepted")

    reader = FrameReader(max_frame_bytes=16)
    try:
        reader.feed(struct.pack(">I", 17) + b"x" * 17)
    except FrameError:
        pass
    else:
        raise AssertionError("oversized frame accepted")

    reader = FrameReader()
    try:
        reader.feed(frame(b"not json"))
    except FrameError:
        pass
    else:
        raise AssertionError("non-JSON payload accepted")


# -- end-to-end over a real Unix socket -------------------------------------


def make_client(server, inbound=None, outbound=None, **kw):
    return IpcClient(
        server.address,
        inbound if inbound is not None else CommandQueue(),
        outbound if outbound is not None else OutboundQueue(),
        **kw
    )


def test_client_roundtrip_over_unix_socket():
    server = LoopbackServer.unix()
    inbound = CommandQueue()
    client = make_client(server, inbound)
    try:
        assert client.connect()
        assert client.is_connected

        # Outbound: client -> server.
        assert client.send_message({"direction": "up", "n": 1})
        assert server.recv_frame() == {"direction": "up", "n": 1}

        # Inbound: server -> client -> inbound queue.
        server.send_message({"command_name": "ping"})
        deadline = 50
        while inbound.pending() == 0 and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
        assert inbound.drain() == [{"command_name": "ping"}]

        # Sender bumps `sent` after sendall returns — poll rather than race.
        deadline = 200
        while client.stats()["sent"] < 1 and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
        stats = client.stats()
        assert stats["sent"] == 1
        assert stats["received"] == 1
        assert stats["protocol_errors"] == 0
    finally:
        client.close()
        server.close()


def test_client_roundtrip_over_tcp_loopback():
    # TCP is the documented debugging/fallback transport; the same client
    # must serve both families.
    server = LoopbackServer.tcp()
    client = make_client(server)
    try:
        assert client.connect()
        assert client.send_message({"hello": "tcp"})
        assert server.recv_frame() == {"hello": "tcp"}
    finally:
        client.close()
        server.close()


def test_receiver_thread_only_enqueues_never_touches_engine():
    # §8.3 #1: inbound IPC traffic must not call the engine scripting API;
    # it lands in the queue for the pump (scripting thread) to consume.
    engine, bootstrap = make_bootstrap(ipc="unused-here")
    server = LoopbackServer.unix()
    client = IpcClient(
        server.address, bootstrap.queue, OutboundQueue(DEFAULT_OUTBOUND_LIMIT)
    )
    try:
        scheduled_before = dict(engine.scheduled)
        assert client.connect()
        for i in range(5):
            server.send_message({"command_name": "c%d" % i})
        deadline = 100
        while bootstrap.queue.pending() < 5 and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
        assert bootstrap.queue.pending() == 5
        assert engine.scheduled == scheduled_before, \
            "receiver thread touched the engine"

        # The pump consumes them FIFO on the scripting thread.
        bootstrap.on_init(0)
        engine.fire(bootstrap.pump_id)
        assert bootstrap._events == [
            {"command_name": "c%d" % i} for i in range(5)
        ]
    finally:
        client.close()
        server.close()


def test_protocol_error_drops_connection_and_counts():
    server = LoopbackServer.unix()
    inbound = CommandQueue()
    client = make_client(server, inbound)
    try:
        assert client.connect()
        # An oversized frame is a framing violation: the stream boundary is
        # unknown afterwards, so the client must drop the connection rather
        # than resync or buffer unboundedly.
        server.send_raw(struct.pack(">I", 10 * 1024 * 1024) + b"junk")
        deadline = 200
        while client.is_connected and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
        assert not client.is_connected
        assert client.stats()["protocol_errors"] == 1
        assert inbound.pending() == 0
    finally:
        client.close()
        server.close()


def test_inbound_overflow_drops_and_counts():
    server = LoopbackServer.unix()
    inbound = CommandQueue(max_size=2)
    client = make_client(server, inbound)
    try:
        assert client.connect()
        for i in range(5):
            server.send_message({"n": i})
        deadline = 200
        while client.stats()["received"] + client.stats()["dropped_inbound"] < 5 \
                and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
        stats = client.stats()
        assert stats["received"] + stats["dropped_inbound"] == 5
        assert stats["dropped_inbound"] >= 3
    finally:
        client.close()
        server.close()


class GatedOutbound(OutboundQueue):
    """OutboundQueue whose first drain parks until `release()`.

    The sender thread starts draining the moment `connect()` returns, so a
    plain queue lets the sender race the test's puts and the overflow never
    deterministically happens. Gating the first drain makes the queueing
    sequence deterministic; release() then lets the sender prove the two
    queued messages actually go out.
    """

    def __init__(self, max_size):
        OutboundQueue.__init__(self, max_size)
        self._gate = threading.Event()
        self._drains = 0

    def release(self):
        self._gate.set()

    def drain(self, timeout=0.05):
        self._drains += 1
        if self._drains == 1:
            self._gate.wait(5.0)
        return OutboundQueue.drain(self, timeout)


def test_outbound_overflow_drops_newest_and_counts():
    server = LoopbackServer.unix()
    outbound = GatedOutbound(max_size=2)
    client = make_client(server, outbound=outbound)
    try:
        assert client.connect()
        # Sender is parked at the gate: the bound is exercised exactly.
        assert outbound.put({"n": 1})
        assert outbound.put({"n": 2})
        assert not client.send_message({"n": 3}), "newest dropped on overflow"
        assert outbound.overflow_count() == 1
        outbound.release()
        # The queued pair really goes out; the dropped one never does.
        assert server.recv_frame() == {"n": 1}
        assert server.recv_frame() == {"n": 2}
        deadline = 200
        while client.stats()["sent"] < 2 and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
        assert client.stats()["sent"] == 2
    finally:
        client.close()
        server.close()


def test_send_message_counts_unserialisable_and_keeps_running():
    # Serialisation is the sender thread's job (the engine thread never
    # JSON-encodes), so a bad message is reported asynchronously.
    server = LoopbackServer.unix()
    client = make_client(server)
    try:
        assert client.connect()
        assert client.send_message({"bad": object()}), "queued successfully"
        deadline = 200
        while client.stats()["encode_errors"] < 1 and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
        assert client.stats()["encode_errors"] == 1, "sender counted it"
        assert client.is_connected, "one bad message must not kill the link"
        assert client.send_message({"ok": True})
        assert server.recv_frame() == {"ok": True}
        # The sender counts `sent` after sendall returns, which can land just
        # after the server already has the bytes: poll, don't race.
        deadline = 200
        while client.stats()["sent"] < 1 and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
        assert client.stats()["sent"] == 1
    finally:
        client.close()
        server.close()


def test_connect_failure_returns_false_and_counts():
    client = IpcClient(
        "/nonexistent-dir-should-fail/x.sock",
        CommandQueue(),
        OutboundQueue(DEFAULT_OUTBOUND_LIMIT),
    )
    assert not client.connect(), "connect must not raise into on_init"
    assert not client.is_connected
    assert client.stats()["connect_errors"] == 1
    assert not client.send_message({"x": 1})
    client.close()


def test_frame_reader_rejects_non_utf8_and_non_object_payloads():
    from bridge.ipc_client import FrameError, FrameReader

    reader = FrameReader()
    try:
        reader.feed(struct.pack(">I", 3) + b"\xff\xfe\xfd")
    except FrameError:
        pass
    else:
        raise AssertionError("non-UTF-8 payload accepted")

    reader = FrameReader()
    try:
        reader.feed(struct.pack(">I", 6) + b"[1, 2]")
    except FrameError:
        pass
    else:
        raise AssertionError("non-object JSON message accepted")


def test_truncated_frame_then_peer_death_ends_session_as_disconnect():
    # A peer that dies mid-frame (partial header, then EOF) must end the
    # session as a disconnect — no hang, no exception, and the buffered
    # partial is discarded with the session instead of leaking forward.
    server = LoopbackServer.unix()
    client = make_client(server)
    try:
        assert client.connect()
        conn = server.wait_connected()
        conn.sendall(b"\x00\x00")  # header cut in half
        server.close()             # then the peer dies
        deadline = 200
        while client.is_connected and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
        assert not client.is_connected, "mid-frame peer death must disconnect"
        assert not client.send_message({"late": True})
        stats = client.stats()
        assert stats["protocol_errors"] == 0, "EOF is a disconnect, not garbage"
    finally:
        client.close()


def test_peer_disconnect_surfaces_as_state_not_exception():
    server = LoopbackServer.unix()
    client = make_client(server)
    try:
        assert client.connect()
        server.close()  # peer vanishes mid-session
        deadline = 200
        while client.is_connected and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
        assert not client.is_connected
        assert not client.send_message({"late": True})
    finally:
        client.close()


def test_close_joins_threads_is_idempotent_and_accounts_every_message():
    # The sender races the close, so each queued message ends up in exactly
    # one of two places: sent to the peer, or counted as dropped on close.
    # The invariant under test is that nothing is silently lost either way.
    queued = 2
    server = LoopbackServer.unix()
    outbound = OutboundQueue(max_size=8)
    client = make_client(server, outbound=outbound)
    try:
        assert client.connect()
        for i in range(queued):
            outbound.put({"q": i})
        assert client.close(timeout=2.0), "both threads exited in time"
        assert client.close(), "second close is a no-op"
        assert not client.is_connected
        stats = client.stats()
        assert stats["sent"] + stats["dropped_outbound"] == queued
        assert stats["pending_outbound"] == 0, "queue drained on close"
        # Sending after close reports False instead of raising.
        assert not client.send_message({"after": "close"})
    finally:
        server.close()


def test_connect_is_single_winner_under_race():
    server = LoopbackServer.unix()
    client = make_client(server)
    try:
        results = []
        threads = [
            threading.Thread(target=lambda: results.append(client.connect()))
            for _ in range(4)
        ]
        for t in threads:
            t.start()
        for t in threads:
            t.join(5)
        assert results == [True, True, True, True]
        assert client.is_connected
        assert client.send_message({"single": "socket"})
        assert server.recv_frame() == {"single": "socket"}
    finally:
        client.close()
        server.close()


# -- bootstrap integration --------------------------------------------------


def test_bootstrap_standby_has_no_client():
    engine, b = make_bootstrap()
    b.on_init(0)
    assert b.client is None
    assert b.ipc_stats() is None
    assert not b.send_message({"x": 1}), "standby cannot send"
    b.on_finish()


def test_bootstrap_wires_ipc_client_lifecycle():
    server = LoopbackServer.unix()
    config = BootstrapConfig(
        ipc_address=server.address, token="tok", swell_id="sw", backend_id="be"
    )
    fe = FakeEngine()
    b = Bootstrap(EnginePort(fe), config)
    try:
        assert b.on_init(0) == "running"
        assert b.client is not None
        assert b.client.is_connected
        assert b.on_init(0) == "running", "second init guarded (S6), returns state"

        # WM-0203: the handshake is the automatic first outbound frame.
        first = server.recv_frame()
        assert first["kind"] == "handshake"
        assert first["protocol_version"] == PROTOCOL_VERSION
        assert first["backend_id"] == "be"

        # Outbound path through the bootstrap.
        assert b.send_message({"kind": "get_status"})
        assert server.recv_frame()["kind"] == "get_status"

        # Inbound path reaches the pump through the shared queue.
        server.send_message({"command_name": "apply_plan"})
        deadline = 100
        while b.queue.pending() == 0 and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
        assert b.queue.pending() == 1

        # Poll: the sender's `sent` bump can lag the server's frame receipt.
        deadline = 200
        stats = b.ipc_stats()
        while (stats["sent"] < 2 or stats["received"] < 1) and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
            stats = b.ipc_stats()
        assert stats["sent"] == 2 and stats["received"] == 1

        # on_finish is synchronous and tears the client down inline.
        assert b.on_finish() == "stopped"
        assert b.client is None
        assert b.ipc_stats() is None
        assert not b.send_message({"x": 1})
        assert not fe.scheduled, "pump cancelled"
    finally:
        server.close()


def test_bootstrap_on_init_survives_unreachable_ipc():
    # An unreachable Supervisor must not abort engine startup (audit §5:
    # an onInit exception kills the bots process).
    config = BootstrapConfig(
        ipc_address="/nonexistent-dir-should-fail/x.sock", backend_id="be"
    )
    fe = FakeEngine()
    b = Bootstrap(EnginePort(fe), config)
    assert b.on_init(0) == "running", "connect failure swallowed"
    assert b.client is not None
    assert not b.client.is_connected
    assert b.client.stats()["connect_errors"] == 1
    assert len(fe.scheduled) == 1, "pump still armed"
    assert b.on_finish() == "stopped"


# -- WM-0203: handshake and capability report -------------------------------


def ack(code="OK", generation=7, reason_code=""):
    """Supervisor-side handshake ack, mirroring EngineHandshakeAck."""
    result = {"code": code}
    if reason_code:
        result["reason_code"] = reason_code
    return {"kind": HANDSHAKE_ACK_KIND, "result": result, "generation": generation}


def test_handshake_payload_mirrors_proto_and_capability_report():
    report = {
        "protocol_version": PROTOCOL_VERSION,
        "engine_name": "kbengine",
        "engine_version": "9.9-test",
        "adapter_name": "wavemaker-kbengine-bridge",
        "adapter_version": "0.1.0",
        "backend_id": "be-1",
        "backend_generation": 12,
        "capabilities": CAPABILITIES,
        "scheduling_resolution_ms": 100,
        "arrival_batch_ms": 1000,
    }
    hs = build_handshake(report)
    assert hs["kind"] == "handshake"
    assert hs["protocol_version"] == PROTOCOL_VERSION == 1
    assert hs["capabilities"] is CAPABILITIES, "honest catalog by reference"
    assert hs["engine_version"] == "9.9-test"
    # Launch hint rides the additive extension field; the ack's generation
    # stays authoritative (WM-0205).
    assert hs["backend_generation"] == 12
    # No invented fields: everything comes from the report.
    assert set(hs) == set(report) | {"kind"}


def test_parse_ack_accepts_only_ok_and_shaped_messages():
    assert parse_handshake_ack(ack()) == (True, 7, "")
    refused = parse_handshake_ack(ack("UNSUPPORTED", 0, "protocol_version"))
    assert refused == (False, 0, "protocol_version")
    # Non-ack messages -> None (caller records; S3: never raises).
    assert parse_handshake_ack({"kind": "apply_plan"}) is None
    assert parse_handshake_ack("garbage") is None
    # Malformed result block: refused with a stable local reason.
    assert parse_handshake_ack({"kind": HANDSHAKE_ACK_KIND, "result": "x"}) == (
        False,
        0,
        "malformed_ack",
    )


def test_handshake_exchange_accepts_to_ready():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address, backend="be-3", generation=6)
    try:
        b.on_init(0)
        hs = server.recv_frame()
        assert hs["kind"] == "handshake"
        assert hs["backend_generation"] == 6, "launch hint on the wire (WM-0205)"
        assert hs["capabilities"]["scheduling"]["exact_arrival"] == "degraded"

        server.send_message(ack(generation=42))
        deadline = 100
        while b.queue.pending() == 0 and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
        engine.fire(b.pump_id)  # pump consumes the ack on the scripting thread
        assert b.state == "ready"
        assert b.handshake_generation == 42
        assert len(engine.scheduled) == 1, "pump keeps running in ready"
    finally:
        b.on_finish()
        server.close()


def test_handshake_refusal_refuses_startup_without_crashing_engine():
    # docs/bridge.md: version mismatch refuses startup — no silent
    # degradation. The bridge goes inert; the engine keeps running (audit §5).
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address)
    try:
        b.on_init(0)
        assert server.recv_frame()["kind"] == "handshake"
        server.send_message(ack("UNSUPPORTED", 0, "protocol_version"))
        deadline = 100
        while b.queue.pending() == 0 and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
        engine.fire(b.pump_id)
        assert b.state == "refused"
        assert b.refusal_reason == "protocol_version"
        assert b.client is None, "session torn down"
        assert not b.send_message({"x": 1}), "no traffic after refusal"
        assert not engine.scheduled, "pump stopped: nothing can flow"
        # The engine is untouched and finish still works.
        assert b.on_finish() == "stopped"
    finally:
        server.close()


def test_unknown_inbound_messages_are_recorded_not_fatal():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address)
    try:
        b.on_init(0)
        server.recv_frame()  # drain the handshake
        server.send_message({"kind": "something_new", "v": 1})
        deadline = 100
        while b.queue.pending() == 0 and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
        engine.fire(b.pump_id)
        assert b.state == "running", "unknown kinds never move the machine"
        assert b._events == [{"kind": "something_new", "v": 1}]
        assert len(engine.scheduled) == 1, "pump survives (S3 discipline)"
    finally:
        b.on_finish()
        server.close()


# -- command dispatch (WM-0206) --------------------------------------------

def command(op="op-1", generation=42, name="start", payload=None):
    return {
        "kind": "command",
        "operation_id": op,
        "generation": generation,
        "command_name": name,
        "payload": payload,
    }


def wait_command(b, engine):
    """Wait for an inbound command to reach the queue, then pump it."""
    deadline = 100
    while b.queue.pending() == 0 and deadline:
        deadline -= 1
        threading.Event().wait(0.01)
    engine.fire(b.pump_id)


def complete_handshake(server, engine, b, generation=42):
    b.on_init(0)
    server.recv_frame()  # drain the handshake
    server.send_message(ack(generation=generation))
    wait_command(b, engine)
    assert b.state == "ready"


def test_parse_command_shapes_mirroring_proto():
    assert parse_command(command(payload={"shard": [1, 2]})) == (
        "op-1", 42, "start", {"shard": [1, 2]},
    )
    # Proto3 defaults for missing fields; only `kind` identifies the message.
    assert parse_command({"kind": "command"}) == ("", 0, "", None)
    # Non-command shapes -> None (S3: never raises into the pump).
    assert parse_command({"kind": "handshake_ack"}) is None
    assert parse_command({"kind": "command", "generation": True})[1] == 0
    assert parse_command({"kind": "command", "generation": "7"})[1] == 0
    assert parse_command({"kind": "command", "operation_id": 9})[0] == ""
    assert parse_command("garbage") is None
    assert parse_command(None) is None


def test_command_result_mirrors_proto_envelope():
    r = build_command_result("op-9", 42, RESULT_OK)
    assert r["kind"] == "command_result"
    assert r["operation_id"] == "op-9"
    assert r["generation"] == 42
    assert r["result"] == {
        "code": "OK", "reason_code": "", "detail": "", "generation": 42,
    }
    err = build_command_result(
        "op-9", 42, RESULT_UNSUPPORTED, "unknown_command", "detail text")
    assert err["result"]["code"] == "UNSUPPORTED"
    assert err["result"]["reason_code"] == "unknown_command"
    assert err["result"]["detail"] == "detail text"


def test_command_before_handshake_is_failed_precondition():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address)
    try:
        b.on_init(0)
        server.recv_frame()  # handshake sent, ack not yet
        server.send_message(command(generation=1))
        wait_command(b, engine)
        result = server.recv_frame()
        assert result["kind"] == "command_result"
        assert result["operation_id"] == "op-1"
        assert result["result"]["code"] == "FAILED_PRECONDITION"
        assert result["result"]["reason_code"] == "not_ready"
        assert b.state == "running", "failed command never moves the machine"
    finally:
        b.on_finish()
        server.close()


def test_stale_generation_command_is_rejected_and_fresh_one_accepted():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address)
    seen = []
    b.set_command_handler("start", lambda p, op: seen.append(p))
    try:
        complete_handshake(server, engine, b, generation=42)
        server.send_message(command(op="old", generation=7))
        wait_command(b, engine)
        result = server.recv_frame()
        assert result["result"]["code"] == "FAILED_PRECONDITION"
        assert result["result"]["reason_code"] == "stale_generation"
        assert result["generation"] == 42, "result carries assigned generation"
        assert seen == [], "stale command never reached the handler"

        server.send_message(command(op="fresh", generation=42, payload={"n": 1}))
        wait_command(b, engine)
        result = server.recv_frame()
        assert result["result"]["code"] == "OK"
        assert seen == [{"n": 1}], "current-generation command dispatched"
    finally:
        b.on_finish()
        server.close()


def test_unknown_command_name_is_unsupported_not_silent():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address)
    try:
        complete_handshake(server, engine, b)
        server.send_message(command(name="nope"))
        wait_command(b, engine)
        result = server.recv_frame()
        assert result["result"]["code"] == "UNSUPPORTED"
        assert result["result"]["reason_code"] == "unknown_command"
    finally:
        b.on_finish()
        server.close()


def test_handler_runs_on_scripting_thread_and_reports_errors():
    # docs/bridge.md §Command Queue: commands execute on the engine
    # callback (scripting) thread; the test thread pumps, so the handler's
    # thread identity must match whoever fired the pump.
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address)
    marker = {}
    b.set_command_handler("start", lambda p, op: marker.update(
        payload=p, thread=threading.get_ident()))
    try:
        complete_handshake(server, engine, b)
        server.send_message(command(payload={"bots": 5}))
        wait_command(b, engine)
        result = server.recv_frame()
        assert result["result"]["code"] == "OK"
        assert marker["payload"] == {"bots": 5}
        assert marker["thread"] == threading.get_ident()

        # A raising handler becomes INTERNAL, never an escaped exception;
        # the pump rearms and the next command still dispatches.
        b.set_command_handler("start", lambda p, op: 1 / 0)
        server.send_message(command(op="op-2"))
        wait_command(b, engine)
        result = server.recv_frame()
        assert result["operation_id"] == "op-2"
        assert result["result"]["code"] == "INTERNAL"
        assert result["result"]["reason_code"] == "handler_error"
        assert "ZeroDivisionError" in result["result"]["detail"]

        b.set_command_handler("start", lambda p, op: None)
        server.send_message(command(op="op-3"))
        wait_command(b, engine)
        assert server.recv_frame()["result"]["code"] == "OK"
        assert len(engine.scheduled) == 1, "pump survived the handler error"
    finally:
        b.on_finish()
        server.close()


# -- ApplyPlan / Start mapping (WM-0207) ------------------------------------

VALID_PLAN = {
    "swell_id": "sw-7",
    "plan_digest": "sha256:abc",
    "batches": [
        {"offset_ms": 0, "bot_count": 10},
        {"offset_ms": 300, "bot_count": 20},
    ],
}


def test_apply_plan_accepts_and_replays_over_the_wire():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address, swell="sw-7")
    try:
        complete_handshake(server, engine, b)
        server.send_message(command(
            op="apply-1", name="apply_plan", payload=VALID_PLAN))
        wait_command(b, engine)
        result = server.recv_frame()
        assert result["result"]["code"] == "OK"
        assert b.plans.state == "plan_accepted"
        assert b.plans.plan_digest == "sha256:abc"

        # Idempotent replay for the same digest (docs/bridge.md).
        server.send_message(command(
            op="apply-2", name="apply_plan", payload=VALID_PLAN))
        wait_command(b, engine)
        assert server.recv_frame()["result"]["code"] == "OK"

        # Conflicting digest is refused: the frozen shard is not replaced.
        conflicting = dict(VALID_PLAN, plan_digest="sha256:zzz")
        server.send_message(command(
            op="apply-3", name="apply_plan", payload=conflicting))
        wait_command(b, engine)
        result = server.recv_frame()
        assert result["result"]["code"] == "FAILED_PRECONDITION"
        assert result["result"]["reason_code"] == "conflicting_plan"
        assert b.plans.plan_digest == "sha256:abc"
    finally:
        b.on_finish()
        server.close()


def test_malformed_plan_payload_is_invalid_argument_not_internal():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address, swell="sw-7")
    try:
        complete_handshake(server, engine, b)
        server.send_message(command(
            op="apply-x", name="apply_plan",
            payload={"swell_id": "sw-7", "plan_digest": "sha256:x",
                     "batches": []}))
        wait_command(b, engine)
        result = server.recv_frame()
        assert result["result"]["code"] == "INVALID_ARGUMENT"
        assert result["result"]["reason_code"] == "batches_missing"
        assert b.plans.state == "idle", "rejected plan stores nothing"
    finally:
        b.on_finish()
        server.close()


def test_start_executes_the_mapped_batches_through_addbots():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address, swell="sw-7")
    try:
        complete_handshake(server, engine, b)
        server.send_message(command(name="start"))
        wait_command(b, engine)
        result = server.recv_frame()
        assert result["result"]["code"] == "FAILED_PRECONDITION"
        assert result["result"]["reason_code"] == "plan_not_accepted"

        server.send_message(command(
            op="apply-1", name="apply_plan", payload=VALID_PLAN))
        wait_command(b, engine)
        assert server.recv_frame()["result"]["code"] == "OK"

        server.send_message(command(op="start-1", name="start"))
        wait_command(b, engine)
        assert server.recv_frame()["result"]["code"] == "OK"

        # Bucket 0 fires on the pump tick that processed start (same-tick
        # due_batches); the explicit second tick adds nothing. Bucket 3
        # (300 ms) timing is covered by the unit tests with a fake clock.
        assert engine.add_bots_calls == [10]
        assert engine.bots_total == 10
        engine.fire(b.pump_id)
        assert engine.add_bots_calls == [10], "one addBots per due batch"

        # Same operation_id replays OK without re-firing the schedule.
        server.send_message(command(op="start-1", name="start"))
        wait_command(b, engine)
        assert server.recv_frame()["result"]["code"] == "OK"
        engine.fire(b.pump_id)
        assert engine.add_bots_calls == [10], "no duplicate addBots"

        # A different operation_id while running is refused.
        server.send_message(command(op="start-2", name="start"))
        wait_command(b, engine)
        result = server.recv_frame()
        assert result["result"]["code"] == "FAILED_PRECONDITION"
        assert result["result"]["reason_code"] == "already_running"
    finally:
        b.on_finish()
        server.close()


def test_stop_admission_over_the_wire_freezes_and_blocks_start():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address, swell="sw-7")
    try:
        complete_handshake(server, engine, b)
        server.send_message(command(
            op="apply-1", name="apply_plan", payload=VALID_PLAN))
        wait_command(b, engine)
        assert server.recv_frame()["result"]["code"] == "OK"
        server.send_message(command(op="start-1", name="start"))
        wait_command(b, engine)
        assert server.recv_frame()["result"]["code"] == "OK"
        assert engine.add_bots_calls == [10]  # bucket 0 fired

        # StopAdmission: idempotent, existing bots untouched (the engine
        # keeps its live clients; add_bots_calls never grows again).
        server.send_message(command(op="stop-1", name="stop_admission"))
        wait_command(b, engine)
        assert server.recv_frame()["result"]["code"] == "OK"
        server.send_message(command(op="stop-2", name="stop_admission"))
        wait_command(b, engine)
        assert server.recv_frame()["result"]["code"] == "OK"

        server.send_message(command(op="start-2", name="start"))
        wait_command(b, engine)
        result = server.recv_frame()
        assert result["result"]["code"] == "FAILED_PRECONDITION"
        assert result["result"]["reason_code"] == "admission_stopped"
        engine.fire(b.pump_id)
        assert engine.add_bots_calls == [10], "schedule frozen, no new bots"
    finally:
        b.on_finish()
        server.close()


def test_drain_over_the_wire_freezes_and_keeps_the_session_live():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address, swell="sw-7")
    try:
        complete_handshake(server, engine, b)
        server.send_message(command(
            op="apply-1", name="apply_plan", payload=VALID_PLAN))
        wait_command(b, engine)
        assert server.recv_frame()["result"]["code"] == "OK"

        server.send_message(command(op="drain-1", name="drain"))
        wait_command(b, engine)
        assert server.recv_frame()["result"]["code"] == "OK"
        assert b.lifecycle.state == "draining"
        assert b.state == "draining", "session stays live for events"
        assert len(engine.scheduled) == 1, "pump re-arms while draining"
        assert b.plans.admission_stopped, "drain implies the admission gate"

        # Idempotent replay of the same operation_id; a different one is
        # refused.
        server.send_message(command(op="drain-1", name="drain"))
        wait_command(b, engine)
        assert server.recv_frame()["result"]["code"] == "OK"
        server.send_message(command(op="drain-2", name="drain"))
        wait_command(b, engine)
        result = server.recv_frame()
        assert result["result"]["code"] == "FAILED_PRECONDITION"
        assert result["result"]["reason_code"] == "already_draining"
    finally:
        b.on_finish()
        server.close()


def test_shutdown_ack_flows_before_inline_teardown():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address)
    try:
        complete_handshake(server, engine, b)
        server.send_message(command(op="down-1", name="shutdown"))
        wait_command(b, engine)
        # The ack is delivered while the session is still up; teardown is
        # deferred to the NEXT pump tick (§8.3 #5: synchronous teardown
        # with one tick of flush time, no async waits).
        assert server.recv_frame()["result"]["code"] == "OK"
        assert b.state == "ready", "not torn down inside the ack tick"
        assert len(engine.scheduled) == 1, "one more tick was armed"

        engine.fire(b.pump_id)
        assert b.state == "stopped"
        assert b.lifecycle.state == "stopped"
        assert b.client is None, "IPC session closed inline"
        assert not engine.scheduled, "pump cancelled, nothing re-armed"
        assert engine.cancelled, "live pump handle was cancelled once"
    finally:
        b.on_finish()  # idempotent on an already-stopped session
        server.close()


def test_shutdown_idempotent_and_repeatable():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address)
    try:
        complete_handshake(server, engine, b)
        for op in ("down-1", "down-2"):
            server.send_message(command(op=op, name="shutdown"))
        wait_command(b, engine)
        # Both acks enqueue; only the teardown matters downstream.
        assert server.recv_frame()["result"]["code"] == "OK"
        assert server.recv_frame()["result"]["code"] == "OK"
        engine.fire(b.pump_id)  # deferred teardown tick
        assert b.state == "stopped"
    finally:
        b.on_finish()
        server.close()


# -- heartbeat and reconnection (WM-0212) ------------------------------------

class ReconnectServer(object):
    """TCP loopback listener that accepts connections one after another on
    the same listening socket — the WM-0212 reconnection counterpart of the
    one-shot LoopbackServer."""

    def __init__(self):
        self._sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._sock.bind(("127.0.0.1", 0))
        self._port = self._sock.getsockname()[1]
        self._sock.listen(1)
        self._closed = False
        self._conn = None
        self._buf = bytearray()
        self.connections = 0
        self._accept_thread = threading.Thread(
            target=self._accept_loop, daemon=True
        )
        self._accept_thread.start()

    def _accept_loop(self):
        while not self._closed:
            try:
                conn, _ = self._sock.accept()
            except OSError:
                return
            conn.settimeout(5.0)
            self._conn = conn
            self.connections += 1

    @property
    def address(self):
        return "tcp:127.0.0.1:%d" % self._port

    def drop_connection(self):
        if self._conn is not None:
            try:
                self._conn.close()
            except OSError:
                pass
            self._conn = None

    def wait_connection(self, timeout=5.0):
        deadline = 500
        while self._conn is None and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
        assert self._conn is not None, "client never (re)connected"

    def recv_frame(self, timeout=5.0):
        self.wait_connection(timeout)
        self._conn.settimeout(timeout)
        while True:
            if len(self._buf) >= 4:
                (length,) = struct.unpack_from(">I", self._buf, 0)
                if len(self._buf) >= 4 + length:
                    payload = bytes(self._buf[4:4 + length])
                    del self._buf[:4 + length]
                    return json.loads(payload.decode("utf-8"))
            chunk = self._conn.recv(65536)
            assert chunk != b"", "client closed the connection"
            self._buf.extend(chunk)

    def send_message(self, message):
        self.wait_connection()
        self._conn.sendall(encode_frame(message))

    def close(self):
        self._closed = True
        for sock in (self._conn, self._sock):
            if sock is not None:
                try:
                    sock.close()
                except OSError:
                    pass


def test_reconnect_restores_session_and_rehandshakes():
    server = ReconnectServer()
    engine, b = make_bootstrap(
        ipc=server.address, reconnect_initial=0.05, reconnect_max=0.2)
    try:
        b.on_init(0)
        assert server.recv_frame()["kind"] == "handshake"
        server.send_message(ack(generation=42))
        wait_command(b, engine)
        assert b.state == "ready"
        assert b.handshake_generation == 42
        assert server.connections == 1

        # The transport dies (Supervisor restart): the reconnect thread
        # recovers on its own; the scripting thread never blocks.
        server.drop_connection()
        deadline = 500
        while server.connections < 2 and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
        assert server.connections == 2, "reconnect thread restored transport"

        # The pending handshake goes out on a pump tick after the reconnect
        # callback flipped the flag — poll with fires until the flag clears
        # (the callback may already have run before the first fire).
        deadline = 300
        while (b.state != "running" or b._handshake_pending) and deadline:
            deadline -= 1
            engine.fire(b.pump_id)
            threading.Event().wait(0.01)
        assert b.state == "running", "on_reconnect reset the session state"
        assert not b._handshake_pending, "handshake enqueued on a pump tick"
        hs2 = server.recv_frame()
        assert hs2["kind"] == "handshake", "new session re-handshakes"
        assert b.state == "running", "back to handshake-pending"
        assert b.handshake_generation == 0, "old generation is dead"

        server.send_message(ack(generation=43))
        wait_command(b, engine)
        assert b.state == "ready"
        assert b.handshake_generation == 43
        assert b.ipc_stats()["reconnects"] == 1
    finally:
        b.on_finish()
        server.close()


def test_supervisor_appearing_after_on_init_gets_the_handshake():
    path = os.path.join(tempfile.mkdtemp(prefix="wm-late-"), "late.sock")
    engine, b = make_bootstrap(
        ipc=path, reconnect_initial=0.05, reconnect_max=0.2)
    server = None
    try:
        b.on_init(0)
        assert not b.client.is_connected, "no listener yet"
        assert len(engine.scheduled) == 1, "pump alive without a session"
        assert b.state == "running"

        server = LoopbackServer.unix(path)  # the Supervisor listener appears
        deadline = 500
        while not b.client.is_connected and deadline:
            deadline -= 1
            threading.Event().wait(0.01)
        assert b.client.is_connected, "reconnect thread found the listener"

        # The pending handshake goes out on a pump tick after the reconnect
        # callback flipped the flag — poll with fires until it clears.
        deadline = 300
        while b._handshake_pending and deadline:
            deadline -= 1
            engine.fire(b.pump_id)
            threading.Event().wait(0.01)
        assert not b._handshake_pending, "handshake enqueued on a pump tick"
        assert server.recv_frame()["kind"] == "handshake"
        server.send_message(ack(generation=9))
        wait_command(b, engine)
        assert b.state == "ready"
        assert b.handshake_generation == 9
    finally:
        b.on_finish()
        if server is not None:
            server.close()


def test_refusal_closes_session_and_stops_reconnection():
    server = ReconnectServer()
    engine, b = make_bootstrap(
        ipc=server.address, reconnect_initial=0.05, reconnect_max=0.2)
    try:
        b.on_init(0)
        assert server.recv_frame()["kind"] == "handshake"
        server.send_message(ack("UNSUPPORTED", 0, "protocol_version"))
        wait_command(b, engine)
        assert b.state == "refused"
        assert b.client is None, "refusal tore the session down"

        server.drop_connection()
        threading.Event().wait(0.15)  # any stray reconnect cycle would show
        assert server.connections == 1, "no reconnect after refusal"
        assert not b._session_wanted()
    finally:
        b.on_finish()
        server.close()


def test_heartbeat_keeps_the_session_visibly_alive():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address)
    try:
        complete_handshake(server, engine, b, generation=42)
        for _ in range(Bootstrap.HEARTBEAT_TICKS):
            engine.fire(b.pump_id)
        beat = server.recv_frame()
        assert beat["kind"] == "heartbeat"
        assert beat["sequence"] == 1
        assert beat["generation"] == 42
        assert beat["bridge_state"] == "ready"
        assert isinstance(beat["ipc"], dict) and "sent" in beat["ipc"]
        # WM-0214: process-level metrics ride the beat (None fields when
        # /proc is unavailable — never fake zeros).
        assert set(beat["process"]) == {"cpu_seconds", "rss_bytes",
                                        "fd_count"}
        assert beat["process"]["rss_bytes"] is None or \
            beat["process"]["rss_bytes"] > 0
        # WM-0215: bridge queue depth + callback lag ride the beat too.
        assert set(beat["metrics"]) == {"queue_depth", "queue_dropped",
                                        "callback_lag_ms",
                                        "callback_lag_max_ms"}
        assert beat["metrics"]["queue_depth"] == 0, \
            "the tick drained the queue before the heartbeat"

        for _ in range(Bootstrap.HEARTBEAT_TICKS):
            engine.fire(b.pump_id)
        assert server.recv_frame()["sequence"] == 2, "sequence advances"
    finally:
        b.on_finish()
        server.close()


# -- event sequencing and cursor replay (WM-0213) ---------------------------

from bridge.connection_events import EVENT_BOT_CREATED  # noqa: E402

SHARD_PLAN = dict(VALID_PLAN, plan_digest="sha256:shard",
                  shard_id="shard-3")


def recv_events_batch(server):
    """Next `events` message from the wire, skipping heartbeat frames."""
    while True:
        message = server.recv_frame()
        if message.get("kind") == "events":
            return message


def test_connection_events_flow_as_sequenced_batches_over_the_wire():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address, swell="sw-7",
                               backend="kb-1")
    try:
        complete_handshake(server, engine, b)
        engine.fire(b.pump_id)  # baseline tick (no events yet)
        engine.bots.add(501)
        engine.fire(b.pump_id)  # diff tick -> unsolicited batch
        batch = recv_events_batch(server)
        assert batch["backend_id"] == "kb-1"
        assert batch["cursor"] == 1, "cursor is the batch's max sequence"
        envelope = batch["events"][0]
        for field in ("event_id", "backend_id", "swell_id", "shard_id",
                      "bot_id", "sequence", "source_timestamp",
                      "observed_timestamp", "event_type", "payload",
                      "schema_version"):
            assert field in envelope, f"envelope lacks {field}"
        assert envelope["event_id"] == "kb-1:1"
        assert envelope["sequence"] == 1
        assert envelope["swell_id"] == "sw-7"
        assert envelope["shard_id"] == "", "no plan accepted -> empty shard"
        assert envelope["bot_id"] == 501
        assert envelope["event_type"] == EVENT_BOT_CREATED
        assert envelope["schema_version"] == 1
    finally:
        b.on_finish()
        server.close()


def test_subscribe_events_replays_from_cursor_with_stable_ids():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address, swell="sw-7",
                               backend="kb-1")
    try:
        complete_handshake(server, engine, b)
        engine.bots.add(1)
        engine.fire(b.pump_id)
        engine.bots.add(2)
        engine.fire(b.pump_id)
        first = recv_events_batch(server)
        second = recv_events_batch(server)
        assert [e["event_id"] for e in first["events"]] == ["kb-1:1"]
        assert [e["event_id"] for e in second["events"]] == ["kb-1:2"]

        # The Supervisor resumes from its cursor after a reconnect
        # (docs/bridge.md): replay carries the identical stored envelope,
        # so Supervisor-side dedup by event_id collapses the overlap.
        server.send_message(command(
            op="sub-1", name="subscribe_events", payload={"cursor": 1}))
        wait_command(b, engine)
        assert server.recv_frame()["result"]["code"] == "OK"
        replay = recv_events_batch(server)
        assert [e["event_id"] for e in replay["events"]] == ["kb-1:2"]
        assert replay["cursor"] == 2
        assert replay["events"][0] == second["events"][0]
        assert "gap" not in replay and "oldest_available" not in replay

        # Repeating the same cursor replays the same event_ids again.
        server.send_message(command(
            op="sub-2", name="subscribe_events", payload={"cursor": 1}))
        wait_command(b, engine)
        assert server.recv_frame()["result"]["code"] == "OK"
        again = recv_events_batch(server)
        assert [e["event_id"] for e in again["events"]] == ["kb-1:2"]
        assert again["events"][0] == second["events"][0]
    finally:
        b.on_finish()
        server.close()


def test_subscribe_events_defaults_to_the_window_start():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address, swell="sw-7",
                               backend="kb-1")
    try:
        complete_handshake(server, engine, b)
        engine.fire(b.pump_id)  # baseline
        # Missing cursor -> 0 -> everything buffered is replayed; on an
        # empty stream this is a plain position ack, not a gap.
        server.send_message(command(op="sub-1", name="subscribe_events"))
        wait_command(b, engine)
        assert server.recv_frame()["result"]["code"] == "OK"
        batch = recv_events_batch(server)
        assert batch["events"] == []
        assert batch["cursor"] == 0
        assert "gap" not in batch and "oldest_available" not in batch
    finally:
        b.on_finish()
        server.close()


def test_subscribe_events_rejects_a_bad_cursor():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address, swell="sw-7")
    try:
        complete_handshake(server, engine, b)
        server.send_message(command(
            op="sub-bad", name="subscribe_events", payload={"cursor": -1}))
        wait_command(b, engine)
        result = server.recv_frame()
        assert result["result"]["code"] == "INVALID_ARGUMENT"
        assert result["result"]["reason_code"] == "bad_cursor"
    finally:
        b.on_finish()
        server.close()


def test_events_carry_the_accepted_plan_shard_attribution():
    server = LoopbackServer.unix()
    engine, b = make_bootstrap(ipc=server.address, swell="sw-7",
                               backend="kb-1")
    try:
        complete_handshake(server, engine, b)
        server.send_message(command(
            op="apply-1", name="apply_plan", payload=SHARD_PLAN))
        wait_command(b, engine)
        assert server.recv_frame()["result"]["code"] == "OK"
        server.send_message(command(op="start-1", name="start"))
        wait_command(b, engine)
        assert server.recv_frame()["result"]["code"] == "OK"
        # addBots is async on the real engine (creation gate S8); the fake
        # mirrors that by only recording the call, so bots surface here
        # explicitly and the next tick's poll diffs them.
        for i in range(3):
            engine.bots.add(100 + i)
        engine.fire(b.pump_id)
        batch = recv_events_batch(server)
        events = batch["events"]
        assert [e["sequence"] for e in events] == [1, 2, 3]
        assert batch["cursor"] == 3
        assert all(e["shard_id"] == "shard-3" for e in events), \
            "attribution comes from the accepted plan"
        assert all(e["event_id"].startswith("kb-1:") for e in events)
    finally:
        b.on_finish()
        server.close()

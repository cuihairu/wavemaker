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
    return fe, Bootstrap(EnginePort(fe), config)


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
    def unix(cls):
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

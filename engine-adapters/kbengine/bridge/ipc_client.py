"""Local IPC client for the KBEngine Bridge (WM-0202).

Wire format: `[u32 BE length][UTF-8 JSON payload]` — the same length
prefix as the Rust framing layer (crates/wm-runtime/src/framing.rs).
JSON rather than protobuf because the KBEngine embedded interpreter
ships no protobuf bindings; message field names mirror
`wavemaker.engine.v1` (proto/engine/v1/engine_bridge.proto) so the
Supervisor side (WM-0204+) maps messages 1:1 without a translation
layer. Handshake rejection on protocol mismatch is WM-0203's job; this
module is only the transport.

Address syntax (`WAVEMAKER_BRIDGE_IPC`, docs/bridge.md "Two
Communication Boundaries"):

  * `unix:<path>` or a bare absolute path — Unix Domain Socket (Linux
    default),
  * `tcp:<host>:<port>` — loopback fallback for debugging.

Threading model (docs/bridge.md "Bridge Threading Model"; audit
§8.3 #1, S1, S2):

    socket --> receiver thread --> inbound queue --> engine pump
                                                   (scripting thread)
    engine thread --> outbound queue --> sender thread --> socket

The receiver thread never calls into the engine scripting API: it only
enqueues parsed messages (§8.3 #1; S1 makes cross-thread script calls
undefined behaviour). `send_message` never writes the socket from the
caller's thread: a full kernel buffer would stall the engine main loop
(S2), so outbound messages go through a bounded queue drained by a
sender thread; overflow drops the newest message and is counted.

Heartbeat and reconnection policy is WM-0212. This module owns
connect / framing / routing / close and reports connection state and
error counters for that layer to build on; it does not retry on its
own, and a lost connection never raises into the caller.
"""

import json
import select
import socket
import struct
import threading

# A frame larger than this cannot be legitimate (handshake payloads and
# command batches are far below it): refusing the connection beats
# buffering attacker- or garbage-controlled lengths unboundedly.
MAX_FRAME_BYTES = 4 * 1024 * 1024

# Default bound for the outbound queue: enough to absorb a metrics batch
# spike, small enough that a wedged peer surfaces as drops instead of
# unbounded memory inside the engine process.
DEFAULT_OUTBOUND_LIMIT = 1024


class IpcAddressError(ValueError):
    """The injected IPC address is not parseable."""


class FrameError(ValueError):
    """The peer violated the framing contract.

    A length-prefixed stream cannot be resynchronised once a boundary is
    unknown, so a FrameError is terminal: the connection is dropped.
    """


def parse_address(address):
    """Parse an IPC address into ("unix", path) or ("tcp", host, port).

    A bare string starting with "/" is a Unix socket path (the README
    configuration table's default form, e.g. `/tmp/wm.sock`); explicit
    `unix:` and `tcp:` prefixes are also accepted. Anything else raises
    `IpcAddressError` rather than guessing.
    """
    addr = (address or "").strip()
    if not addr:
        raise IpcAddressError("empty IPC address")
    if addr.startswith("unix:"):
        path = addr[len("unix:"):].strip()
        if not path:
            raise IpcAddressError("unix: prefix with no path: %r" % address)
        return ("unix", path)
    if addr.startswith("tcp:"):
        rest = addr[len("tcp:"):].strip()
        host, _, port = rest.rpartition(":")
        if not host or not port:
            raise IpcAddressError("tcp: needs host:port, got %r" % address)
        try:
            return ("tcp", host, int(port))
        except ValueError:
            raise IpcAddressError("tcp: port is not an int: %r" % address)
    if addr.startswith("/"):
        return ("unix", addr)
    raise IpcAddressError(
        "unsupported IPC address %r (use unix:<path>, a bare /path, "
        "or tcp:<host>:<port>)" % address
    )


def encode_frame(message):
    """Encode one message dict as a length-prefixed UTF-8 JSON frame.

    Raises ValueError when the message is not JSON-serialisable; the
    sender thread counts that as an encode error and keeps running.
    """
    payload = json.dumps(message, separators=(",", ":")).encode("utf-8")
    return struct.pack(">I", len(payload)) + payload


class FrameReader(object):
    """Stateful incremental decoder for the length-prefixed JSON stream.

    Feed it whatever `recv` returned (possibly a half frame, possibly
    several frames coalesced) and it returns the complete messages in
    order, buffering the remainder — the same half-frame discipline as
    the Rust `LengthPrefixed` decoder.
    """

    def __init__(self, max_frame_bytes=MAX_FRAME_BYTES):
        self._max = max_frame_bytes
        self._buf = bytearray()

    def feed(self, chunk):
        """Consume bytes, return every complete message dict in order."""
        self._buf.extend(chunk)
        messages = []
        while True:
            if len(self._buf) < 4:
                break
            (length,) = struct.unpack_from(">I", self._buf, 0)
            if length == 0 or length > self._max:
                raise FrameError("frame length %d out of range" % length)
            if len(self._buf) < 4 + length:
                break
            payload = bytes(self._buf[4:4 + length])
            del self._buf[:4 + length]
            try:
                message = json.loads(payload.decode("utf-8"))
            except (UnicodeDecodeError, ValueError):
                raise FrameError("payload is not valid UTF-8 JSON")
            if not isinstance(message, dict):
                raise FrameError("message is not a JSON object")
            messages.append(message)
        return messages


class OutboundQueue(object):
    """Bounded FIFO with a blocking drain, for the sender thread.

    `put` is safe from any thread and never blocks: when full, the newest
    message is dropped and counted (the engine thread must never stall on
    a wedged peer, S2). `drain` blocks up to `timeout` seconds for the
    first item, then takes whatever else is already queued; `wake` ends a
    waiting drain immediately so `close` can join the sender promptly.
    """

    def __init__(self, max_size=DEFAULT_OUTBOUND_LIMIT):
        self._items = []
        self._cond = threading.Condition()
        self._max_size = max_size
        self._overflow = 0

    def put(self, message):
        """Queue an outbound message. Returns False if it was dropped."""
        with self._cond:
            if self._max_size and len(self._items) >= self._max_size:
                self._overflow += 1
                return False
            self._items.append(message)
            self._cond.notify()
            return True

    def drain(self, timeout=0.05):
        """Wait up to `timeout` for the first message, then take all."""
        with self._cond:
            if not self._items:
                self._cond.wait(timeout)
            if not self._items:
                return []
            items = self._items
            self._items = []
            return items

    def wake(self):
        """Interrupt a waiting drain (used by close)."""
        with self._cond:
            self._cond.notify()

    def put_back(self, items):
        """Prepend already-drained items (sender shutdown path).

        Runs only when the sender is exiting and `close` is about to drain
        everything into the dropped counter, so exceeding the bound here is
        harmless — the point is that no message vanishes without being
        counted somewhere.
        """
        with self._cond:
            self._items[:0] = list(items)
            self._cond.notify()

    def pending(self):
        with self._cond:
            return len(self._items)

    def overflow_count(self):
        with self._cond:
            return self._overflow


class IpcClient(object):
    """Local IPC client: connect, frame, route, close.

    One client per Bridge instance. `connect()` returns False instead of
    raising — `Bootstrap.on_init` must not abort engine startup over a
    missing Supervisor (audit §5: an onInit exception kills bots startup)
    — and after connect nothing in this class raises into the caller:
    failures are counted in `stats()` and surface as `is_connected()`.
    """

    def __init__(self, address, inbound_queue, outbound_queue=None,
                 max_frame_bytes=MAX_FRAME_BYTES, poll_seconds=0.05):
        self._address = address
        self._inbound = inbound_queue
        self._outbound = outbound_queue if outbound_queue is not None \
            else OutboundQueue()
        self._max_frame = max_frame_bytes
        self._poll = poll_seconds
        self._sock = None
        self._connected = False
        self._stop = False
        self._receiver = None
        self._sender = None
        # _lock guards state + counters; _connect_lock serialises connect so
        # concurrent callers open exactly one socket.
        self._lock = threading.Lock()
        self._connect_lock = threading.Lock()
        self._stats = {
            "sent": 0,
            "received": 0,
            "dropped_inbound": 0,
            "dropped_outbound": 0,
            "encode_errors": 0,
            "protocol_errors": 0,
            "recv_errors": 0,
            "send_errors": 0,
            "connect_errors": 0,
        }

    # -- lifecycle -----------------------------------------------------------

    def connect(self):
        """Open the connection and start both threads. Returns success."""
        with self._connect_lock:
            with self._lock:
                if self._sock is not None:
                    return self._connected
            try:
                sock = self._open_socket()
            except (OSError, ValueError):
                self._bump("connect_errors")
                return False
            with self._lock:
                if self._sock is not None:
                    # Unreachable while _connect_lock is held, but keep the
                    # guard honest if that ever changes.
                    try:
                        sock.close()
                    except OSError:
                        pass
                    return self._connected
                self._sock = sock
                self._stop = False
                self._connected = True
                self._receiver = threading.Thread(
                    target=self._receive_loop, name="wm-bridge-receiver"
                )
                self._sender = threading.Thread(
                    target=self._send_loop, name="wm-bridge-sender"
                )
                self._receiver.daemon = True
                self._sender.daemon = True
                self._receiver.start()
                self._sender.start()
            return True

    def close(self, timeout=2.0):
        """Synchronously stop both threads and close the socket.

        Safe to call twice; returns True when both threads exited within
        `timeout`. Outbound messages still queued are dropped (and counted)
        — finishing is `onFinish`'s synchronous business, never an async
        teardown (§8.3 #5).
        """
        with self._connect_lock:
            with self._lock:
                if self._sock is None:
                    return True
                self._stop = True
            self._outbound.wake()
            for thread in (self._receiver, self._sender):
                if thread is not None:
                    thread.join(timeout)
            with self._lock:
                sock, self._sock = self._sock, None
                self._connected = False
                # Whatever the sender never got to is dropped, and counted
                # once — the queue is emptied so stats() stays accurate.
                self._stats["dropped_outbound"] += len(self._outbound.drain(0.0))
                exited = (
                    (self._receiver is None or not self._receiver.is_alive())
                    and (self._sender is None or not self._sender.is_alive())
                )
            if sock is not None:
                try:
                    sock.close()
                except OSError:
                    pass
            return exited

    # -- outbound ------------------------------------------------------------

    def send_message(self, message):
        """Queue a message for sending. Returns False when disconnected or
        dropped on overflow. Never raises and never blocks on the socket
        (S2).

        Serialisation happens on the sender thread, not here: the engine
        thread must not spend its tick budget JSON-encoding large
        snapshots. An unserialisable message is therefore reported
        asynchronously as `encode_errors` in `stats()`, and this call
        still returns True for having queued it.
        """
        with self._lock:
            connected = self._connected and not self._stop
        if not connected:
            return False
        return self._outbound.put(message)

    @property
    def is_connected(self):
        with self._lock:
            return self._connected and not self._stop

    def stats(self):
        """Snapshot of the transport counters."""
        with self._lock:
            snapshot = dict(self._stats)
        snapshot["pending_outbound"] = self._outbound.pending()
        snapshot["overflow_outbound"] = self._outbound.overflow_count()
        return snapshot

    # -- internals -----------------------------------------------------------

    def _open_socket(self):
        parsed = parse_address(self._address)
        if parsed[0] == "unix":
            sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            target = parsed[1]
        else:
            sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            target = (parsed[1], parsed[2])
        # Bounded connect: a hung Supervisor must not hang on_init.
        sock.settimeout(2.0)
        sock.connect(target)
        sock.settimeout(None)
        return sock

    def _bump(self, key, by=1):
        with self._lock:
            self._stats[key] += by

    def _mark_disconnected(self):
        with self._lock:
            self._connected = False
            self._stop = True
        self._outbound.wake()

    def _receive_loop(self):
        reader = FrameReader(self._max_frame)
        sock = self._sock
        while True:
            with self._lock:
                if self._stop:
                    return
            try:
                readable, _, _ = select.select([sock], [], [], self._poll)
            except (OSError, ValueError):
                self._bump("recv_errors")
                self._mark_disconnected()
                return
            if not readable:
                continue
            try:
                chunk = sock.recv(65536)
            except OSError:
                self._bump("recv_errors")
                self._mark_disconnected()
                return
            if chunk == b"":
                self._mark_disconnected()
                return
            try:
                messages = reader.feed(chunk)
            except FrameError:
                self._bump("protocol_errors")
                self._mark_disconnected()
                return
            for message in messages:
                # §8.3 #1: the IPC thread only enqueues. Nothing here
                # touches the engine scripting API (S1).
                if self._inbound.enqueue(message):
                    self._bump("received")
                else:
                    self._bump("dropped_inbound")

    def _send_loop(self):
        sock = self._sock
        while True:
            messages = self._outbound.drain(self._poll)
            with self._lock:
                stop = self._stop
            if stop:
                # Give back whatever we drained so close() counts them as
                # dropped — exiting here must not swallow queued messages.
                if messages:
                    self._outbound.put_back(messages)
                return
            for index, message in enumerate(messages):
                try:
                    frame = encode_frame(message)
                except (TypeError, ValueError):
                    # A caller handed us an unserialisable message; count
                    # it and keep the queue moving (S3 discipline: the
                    # loop never dies on one bad item).
                    self._bump("encode_errors")
                    continue
                try:
                    sock.sendall(frame)
                except OSError:
                    self._bump("send_errors")
                    self._outbound.put_back(messages[index + 1:])
                    self._mark_disconnected()
                    return
                self._bump("sent")

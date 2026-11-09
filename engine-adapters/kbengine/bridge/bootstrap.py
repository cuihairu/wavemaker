"""KBEngine Bridge bootstrap: the engine-side shell loaded by `onInit`.

WM-0201. The bootstrap owns:

  * the state machine (standby -> running -> ready/refused -> stopped),
  * the inbound command queue (IPC threads only enqueue; see command_queue),
  * the engine-callback pump that drains the queue on the scripting thread,
  * the injected configuration (Supervisor passes IPC address, token, Swell
    ID, Backend ID — kbengine.md lifecycle step 3),
  * the handshake exchange (WM-0203): the first outbound message after the
    IPC client connects; an OK ack moves the bridge to `ready`, a refusal
    tears the session down (`refused`) without touching engine startup.

IPC transport is owned by the local IPC client (WM-0202): with an injected
address `on_init` starts it and `on_finish` closes it inline; without one the
bootstrap stays in standby, so the engine can load it harmlessly while the
rest of Phase 2 lands.

Constraints honoured here (docs/spec/kbengine-api-audit.md §8.3):

  1. IPC receive threads only enqueue; the pump (on the scripting thread)
     is the sole consumer.
  3. Callback ids are the engine's own; the pump cancels only its current
     live handle on finish (single-shot callbacks never need a cancel of a
     fired handle).
  5. onFinish is synchronous — no async teardown is armed — and the state
     machine never depends on a callback having succeeded (S3: script
     exceptions are printed and swallowed by the engine).
"""

import functools
import os
import threading
import time
import traceback

from .bot_registry import collect_bot_snapshot
from .capability import (
    ARRIVAL_BATCH_MS,
    CAPABILITIES,
    PROTOCOL_VERSION,
    SCHEDULING_RESOLUTION_MS,
)
from .connection_events import ConnectionCollector
from .command_queue import CommandQueue
from .commands import CommandDispatcher, parse_command
from .event_stream import EventStream
from .handshake import build_handshake, parse_handshake_ack
from .ipc_client import DEFAULT_OUTBOUND_LIMIT, IpcClient, OutboundQueue
from .lifecycle import BackendLifecycle
from .metrics import CallbackLagMeter
from .plans import PlanState
from .process_metrics import collect_process_metrics


class BootstrapConfig(object):
    """Supervisor-injected configuration for one Bridge instance.

    The injection channel is environment variables on the bot child process
    (set by the Rust launcher, crates/wm-supervisor/src/kbengine.rs — the
    variable names there are the single source of truth for both sides).
    """

    ENV_IPC = "WAVEMAKER_BRIDGE_IPC"
    ENV_TOKEN = "WAVEMAKER_BRIDGE_TOKEN"
    ENV_SWELL_ID = "WAVEMAKER_SWELL_ID"
    ENV_BACKEND_ID = "WAVEMAKER_BACKEND_ID"
    ENV_BACKEND_GENERATION = "WAVEMAKER_BACKEND_GENERATION"

    def __init__(self, ipc_address="", token="", swell_id="", backend_id="",
                 backend_generation=0):
        self.ipc_address = ipc_address
        self.token = token
        self.swell_id = swell_id
        self.backend_id = backend_id
        # Launch-hint generation (0 = unset). The handshake ack's generation
        # stays authoritative once the Supervisor replies.
        self.backend_generation = backend_generation

    @staticmethod
    def _parse_generation(raw):
        """Decimal u64; anything malformed is 0 (unset) — onInit must never
        raise on bad injection (audit §5: an onInit exception kills bots)."""
        try:
            value = int(str(raw).strip())
        except (TypeError, ValueError):
            return 0
        return value if value > 0 else 0

    @classmethod
    def from_env(cls, environ=None):
        env = os.environ if environ is None else environ
        return cls(
            ipc_address=env.get(cls.ENV_IPC, ""),
            token=env.get(cls.ENV_TOKEN, ""),
            swell_id=env.get(cls.ENV_SWELL_ID, ""),
            backend_id=env.get(cls.ENV_BACKEND_ID, ""),
            backend_generation=cls._parse_generation(
                env.get(cls.ENV_BACKEND_GENERATION)
            ),
        )

    @property
    def has_ipc(self):
        return bool(self.ipc_address)


class Bootstrap(object):
    """Engine-side bridge shell. Construct one per engine process (S6)."""

    # The pump interval is the default engine callback quantum (audit §3):
    # gameUpdateHertz = 10 quantizes delays to 100 ms.
    PUMP_INTERVAL_S = SCHEDULING_RESOLUTION_MS / 1000.0

    # Heartbeat cadence (WM-0212): one beat per 10 pump ticks = 1 s — the
    # engine's own creation-gate cadence (audit §8.1 S8), not a wire
    # requirement. Carries liveness plus transport/lifecycle state.
    HEARTBEAT_TICKS = 10

    def __init__(self, engine, config, queue=None,
                 reconnect_initial_s=1.0, reconnect_max_s=10.0):
        self._engine = engine
        self._config = config
        self._queue = queue if queue is not None else CommandQueue()
        self._pump_id = None
        self._state = "standby"
        self._events = []
        self._client = None
        self._handshake_generation = 0
        self._refusal_reason = ""
        # WM-0212: handshake bookkeeping shared with the reconnect thread.
        # All reads/writes go through _session_lock; the reconnect thread
        # only flips flags — every protocol send stays on the pump.
        self._session_lock = threading.Lock()
        self._handshake_pending = False
        self._reconnect_initial_s = reconnect_initial_s
        self._reconnect_max_s = reconnect_max_s
        self._heartbeat_seq = 0
        self._ticks = 0
        # WM-0215: callback lag of the pump against its armed due instant
        # (the engine may fire late; the excess is reported, not absorbed).
        self._lag = CallbackLagMeter(self.PUMP_INTERVAL_S)
        self._dispatcher = CommandDispatcher(
            self._send_command_result, lambda: self._handshake_generation
        )
        self._plans = PlanState(config.swell_id, SCHEDULING_RESOLUTION_MS)
        self._dispatcher.set_handler("apply_plan", self._on_apply_plan)
        self._dispatcher.set_handler("start", self._on_start)
        self._dispatcher.set_handler("stop_admission", self._on_stop_admission)
        self._dispatcher.set_handler("drain", self._on_drain)
        self._dispatcher.set_handler("shutdown", self._on_shutdown)
        self._dispatcher.set_handler(
            "subscribe_events", self._on_subscribe_events)
        self._connections = ConnectionCollector()
        # WM-0213: event sequencing + replay window. shard_id attribution
        # follows the accepted plan live (events after apply_plan carry it).
        self._event_stream = EventStream(
            config.backend_id, config.swell_id,
            lambda: self._plans.shard_id,
        )
        self._lifecycle = BackendLifecycle()
        self._shutdown_requested = False
        self._shutdown_deferred = False

    @property
    def state(self):
        return self._state

    @property
    def handshake_generation(self):
        """Generation the Supervisor assigned at handshake (0 before it)."""
        return self._handshake_generation

    @property
    def refusal_reason(self):
        """Stable Supervisor reason code after a handshake refusal."""
        return self._refusal_reason

    @property
    def config(self):
        return self._config

    @property
    def queue(self):
        return self._queue

    @property
    def client(self):
        """The IPC client (WM-0202), or None in standby."""
        return self._client

    @property
    def pump_id(self):
        return self._pump_id

    def capability_report(self):
        """Handshake-shaped report (WM-0203 sends it over IPC)."""
        return {
            "protocol_version": PROTOCOL_VERSION,
            "engine_name": "kbengine",
            "engine_version": "unknown",
            "adapter_name": "wavemaker-kbengine-bridge",
            "adapter_version": "0.1.0",
            "backend_id": self._config.backend_id,
            # Launch hint (0 = unset); the ack's generation stays
            # authoritative. Additive extension beyond the frozen proto
            # wire — see handshake.build_handshake.
            "backend_generation": self._config.backend_generation,
            "capabilities": CAPABILITIES,
            "scheduling_resolution_ms": SCHEDULING_RESOLUTION_MS,
            "arrival_batch_ms": ARRIVAL_BATCH_MS,
        }

    # -- engine entry points -------------------------------------------------

    def on_init(self, arg0=0):
        """Entry from the engine entry script's `onInit(arg0: int)`.

        The engine calls this exactly once per process (S6) with a fixed int
        argument 0 (audit §5); a second call is guarded. Throwing here aborts
        engine startup, so this is synchronous and non-throwing by design:
        with an injected IPC address the client is started here, and a
        connect failure is recorded on the client (stats/is_connected)
        rather than raised into the engine (kbengine.md lifecycle step 4).
        """
        if self._state != "standby":
            # Already running or finished: no double init (S6).
            return self._state
        if self._config.has_ipc:
            self._client = IpcClient(
                self._config.ipc_address,
                self._queue,
                OutboundQueue(DEFAULT_OUTBOUND_LIMIT),
                reconnect_backoff=(
                    self._reconnect_initial_s, self._reconnect_max_s),
            )
            self._client.connect()
            # The handshake is the first outbound message (WM-0203); send is
            # a non-blocking enqueue, so on_init stays cheap on the engine
            # thread. A failed connect leaves it unsent — the state stays
            # `running` (handshake pending) and WM-0212's reconnection owns
            # the retry, re-sending the handshake once a session is up.
            if self._client.is_connected:
                self._send_handshake()
            else:
                with self._session_lock:
                    self._handshake_pending = True
            # WM-0212: reconnect runs on its own daemon thread (a bounded
            # connect can block up to 2 s — never on the scripting thread).
            # It stops itself when the session is refused/stopped.
            self._client.start_reconnect(
                should_retry=self._session_wanted,
                on_reconnect=self._on_session_reconnected,
            )
        self._state = "running"
        self._arm_pump()
        return self._state

    def _send_handshake(self):
        """Enqueue the handshake; True when it was queued for sending."""
        message = build_handshake(self.capability_report())
        return self._client.send_message(message)

    def _session_wanted(self):
        """Reconnect gate (WM-0212, runs on the reconnect thread): keep
        retrying only while a protocol session makes sense — never after a
        refusal or shutdown."""
        return self._state in ("running", "ready", "draining")

    def _on_session_reconnected(self):
        """Reconnect-thread callback after a fresh transport session (the
        Supervisor assigns generations per session, so the old one is dead
        regardless of its ack): reset the generation, mark the handshake
        pending and drop back to `running`. The handshake itself goes out
        from the next pump tick — this thread never sends protocol traffic
        (one writer, one enqueue path)."""
        with self._session_lock:
            self._handshake_generation = 0
            self._handshake_pending = True
            if self._state in ("ready", "draining"):
                self._state = "running"

    def on_finish(self):
        """Entry from the engine entry script's `onFinish()` (no args).

        Must be synchronous: the engine's finalise does not wait for async
        teardown (§8.3 #5), so no async work is armed here — the IPC client
        is closed inline (bounded join, never a detached wait) and the pump
        is simply cancelled.
        """
        self._teardown_session()
        return self._state

    def send_message(self, message):
        """Queue a message for the Supervisor over IPC. Returns False when
        in standby, disconnected, or dropped on overflow; never raises and
        never blocks the scripting thread (S2). An unserialisable message is
        reported asynchronously as `encode_errors` in `ipc_stats()` — the
        engine thread never pays for JSON encoding."""
        if self._client is None:
            return False
        return self._client.send_message(message)

    def ipc_stats(self):
        """Transport counters, or None while in standby."""
        if self._client is None:
            return None
        return self._client.stats()

    # -- pump ---------------------------------------------------------------

    def _arm_pump(self):
        # Single-shot (audit §3): re-arm on every drain. The engine forwards
        # no extra args, so functools.partial carries the closure.
        self._pump_id = self._engine.schedule(
            self.PUMP_INTERVAL_S, functools.partial(self._on_pump)
        )
        self._lag.armed()

    def _pump_session_upkeep(self):
        """Send a pending handshake when the transport is up (WM-0212).
        The pending flag clears only after a successful enqueue — a failed
        send (session dropped again) retries on the next tick."""
        if self._client is None:
            return
        with self._session_lock:
            pending = self._handshake_pending
        if pending and self._client.is_connected:
            if self._send_handshake():
                with self._session_lock:
                    self._handshake_pending = False

    def _send_heartbeat(self):
        """One heartbeat (WM-0212): liveness + transport/lifecycle state.
        Non-blocking enqueue; a disconnected session drops it silently (the
        reconnect thread owns recovery). WM-0214: process-level metrics
        ride the same beat (unreadable source -> None fields, never fake
        zeros)."""
        self._heartbeat_seq += 1
        self.send_message({
            "kind": "heartbeat",
            "sequence": self._heartbeat_seq,
            "generation": self._handshake_generation,
            "bridge_state": self._state,
            "ipc": self.ipc_stats(),
            "process": collect_process_metrics(),
            "metrics": self.bridge_metrics(),
            "monotonic": time.monotonic(),
        })

    def bridge_metrics(self):
        """Bridge-level metrics (WM-0215, docs/bridge.md §Metrics Model):
        inbound command queue depth/dropped plus callback lag (the outbound
        depth/dropped already ride the `ipc` transport stats). Each call
        closes the callback-lag max window."""
        metrics = self._lag.snapshot_ms()
        metrics["queue_depth"] = self._queue.pending()
        metrics["queue_dropped"] = self._queue.overflow_count()
        return metrics

    def _teardown_session(self):
        """Inline session teardown (synchronous, bounded): close the IPC
        client and cancel the pump. Used by both the shutdown path (WM-0211)
        and on_finish."""
        if self._client is not None:
            self._client.close()
            self._client = None
        if self._pump_id is not None:
            self._engine.cancel(self._pump_id)
            self._pump_id = None
        self._state = "stopped"

    def _on_pump(self):
        if self._shutdown_deferred:
            # The tick after the shutdown ack was enqueued: tear the session
            # down inline now (the ack had one tick of sender flush time).
            self._shutdown_deferred = False
            self._teardown_session()
            return
        try:
            # WM-0215: how late this fire was against the armed due instant
            # (D4 quantization shows up here instead of being absorbed).
            self._lag.fired()
            self.drain_command_queue()
            # Batch plan execution rides the same pump tick: due batches
            # become one addBots call each, on the scripting thread (S1).
            self.execute_due_batches()
            # Connection events (WM-0209): diff the bots keys() snapshot
            # once per tick; raw records flow through the event stream
            # (WM-0213) into sequenced wire envelopes.
            self._connections.poll(self._engine.bots)
            # WM-0213: the requested replay (subscribe_events) goes out
            # first — it covers the window as of this tick — then this
            # tick's fresh events ride their own batch (no overlap).
            # send_message is a non-blocking enqueue; an unsent batch is
            # dropped but its envelopes stay in the replay window, so the
            # cursor resume remains the durable delivery path.
            replay = self._event_stream.build_replay_if_requested()
            if replay is not None:
                self.send_message(replay)
            batch = self._event_stream.build_batch(
                self._event_stream.assign(self.take_connection_events()))
            if batch is not None:
                self.send_message(batch)
            # WM-0212: the pending handshake goes out here (covers both the
            # post-reconnect re-handshake and a Supervisor that appeared
            # after on_init), and the heartbeat keeps the session visibly
            # alive — all enqueues, S2-safe.
            self._ticks += 1
            self._pump_session_upkeep()
            if self._ticks % self.HEARTBEAT_TICKS == 0:
                self._send_heartbeat()
        except Exception:
            # S3: a script exception is printed and swallowed by the engine;
            # the bridge state machine must not depend on a callback having
            # succeeded, so log and keep the loop alive.
            traceback.print_exc()
        finally:
            if self._shutdown_requested:
                self._shutdown_requested = False
                self._shutdown_deferred = True
            # `ready`/`draining` still have a live protocol session;
            # `refused` has none (the client is torn down and no traffic can
            # flow), so the pump ends there — the engine itself keeps
            # running untouched.
            if self._state in ("running", "ready", "draining"):
                self._arm_pump()

    def drain_command_queue(self):
        """Consume every queued command on the scripting thread. Returns the
        number of commands processed (FIFO order)."""
        commands = self._queue.drain()
        for command in commands:
            self._handle_command(command)
        return len(commands)

    def set_command_handler(self, command_name, handler):
        """Register the handler for one command name (WM-0207+: ApplyPlan /
        Start / StopAdmission / Drain / Shutdown). The handler runs inline on
        the scripting thread when its command is pumped."""
        self._dispatcher.set_handler(command_name, handler)

    def _send_command_result(self, result):
        # Non-blocking enqueue to the IPC sender (S2); False on
        # standby/dropped — the dispatch layer has already built the result,
        # so there is nothing further to do here.
        if self._client is not None:
            self._client.send_message(result)

    # -- ApplyPlan / Start (WM-0207) ----------------------------------------

    def _on_apply_plan(self, payload, operation_id):
        # Structured (code, reason, detail) outcome -> dispatcher envelopes
        # it; validation and digest idempotency live in PlanState.
        return self._plans.apply(payload)

    def _on_start(self, payload, operation_id):
        # Start idempotency keys on operation_id (docs/bridge.md §Command
        # Semantics); the payload carries no Start-relevant fields.
        return self._plans.start(operation_id)

    def _on_stop_admission(self, payload, operation_id):
        # WM-0210: stop new bots, keep existing connections; idempotent.
        return self._plans.stop_admission(operation_id)

    def _on_drain(self, payload, operation_id):
        # WM-0211: mark draining (one-way), freeze the batch schedule (no
        # new bots while draining), keep the session live for events. The
        # bridge cannot close bot connections itself (audit §2: pybots is
        # read-only) — the cleanup judgement runs on the Supervisor against
        # the collected counts/events.
        result = self._lifecycle.drain(operation_id)
        if result[0] == "OK":
            self._plans.stop_admission(operation_id)
            if self._state in ("running", "ready"):
                self._state = "draining"
        return result

    def _on_shutdown(self, payload, operation_id):
        # WM-0211: end the protocol session. The OK result goes out on this
        # tick; the inline teardown runs on the NEXT pump tick so the ack
        # gets a healthy-sender flush window (§8.3 #5: teardown stays
        # synchronous, no async waits). The bots process keeps running —
        # termination is the Supervisor's process-supervision job.
        result = self._lifecycle.shutdown(operation_id)
        if result[0] == "OK":
            self._shutdown_requested = True
        return result

    def _on_subscribe_events(self, payload, operation_id):
        # WM-0213: cursor resume (docs/bridge.md). The cursor is the highest
        # sequence the Supervisor has durably processed; missing/zero
        # replays from the window start. The replay batch itself goes out
        # from the next pump tick (protocol sends stay on the pump).
        cursor = payload.get("cursor", 0) if isinstance(payload, dict) else 0
        if not isinstance(cursor, int) or isinstance(cursor, bool) or cursor < 0:
            return ("INVALID_ARGUMENT", "bad_cursor",
                    "cursor must be a non-negative int")
        self._event_stream.request_replay(cursor)
        return None

    @property
    def lifecycle(self):
        """Backend lifecycle state (active/draining/stopped)."""
        return self._lifecycle

    def execute_due_batches(self):
        """Fire one addBots per due batch on the scripting thread (the pump
        is the only caller). Returns the fired bot counts. addBots returns
        None with no failure feedback (audit §1/D1) — a failed call is
        invisible here, which the Supervisor sees as a bot-count shortfall
        in the WM-0208 collection instead."""
        fired = []
        for count in self._plans.due_batches():
            self._engine.add_bots(count)
            fired.append(count)
        return fired

    @property
    def plans(self):
        """Plan state (accepted digest, running start id) for tests/status."""
        return self._plans

    @property
    def event_stream(self):
        """Event stream (WM-0213): sequence/replay state for tests/status."""
        return self._event_stream

    def bot_snapshot(self):
        """Live bot census (WM-0208): `{bot_count, bot_ids, dead_ids}` on
        the scripting thread, or None while the engine exposes no bots
        mapping (uncollectable — never reported as a fake zero). addBots
        failures are invisible to this side (audit §1/D1); a shortfall
        shows up here as bot_count below the plan target."""
        return collect_bot_snapshot(self._engine.bots)

    def take_connection_events(self):
        """Drain buffered raw connection events (WM-0209 collection; the
        event layer turns these into the sequenced wire envelope,
        WM-0213). Login-state events are not produced: the ClientObject
        state/event surface is runtime-verification pending (audit §2,
        capability catalog keeps login_success unknown)."""
        return self._connections.take_events()

    def _handle_command(self, command):
        ack = parse_handshake_ack(command)
        if ack is not None:
            self._on_handshake_ack(*ack)
            return
        parsed = parse_command(command)
        if parsed is not None:
            # WM-0206: routed to the registered handler (or a stable error
            # result) and the EngineCommandResult goes back over IPC.
            self._dispatcher.dispatch(parsed)
            return
        self._events.append(command)

    def _on_handshake_ack(self, accepted, generation, reason):
        if not accepted:
            # Startup refused (docs/bridge.md): no silent degradation — tear
            # the client down inline (bounded join; pre-bots at this point,
            # so the bounded stall is harmless) and go inert. The engine
            # keeps running; a refusal must never crash bots (audit §5).
            if self._client is not None:
                self._client.close()
                self._client = None
            if self._pump_id is not None:
                self._engine.cancel(self._pump_id)
                self._pump_id = None
            self._state = "refused"
            self._refusal_reason = reason
            return
        with self._session_lock:
            self._handshake_generation = generation
            self._handshake_pending = False
        self._state = "ready"

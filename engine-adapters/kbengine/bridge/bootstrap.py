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
import traceback

from .capability import (
    ARRIVAL_BATCH_MS,
    CAPABILITIES,
    PROTOCOL_VERSION,
    SCHEDULING_RESOLUTION_MS,
)
from .command_queue import CommandQueue
from .commands import CommandDispatcher, parse_command
from .handshake import build_handshake, parse_handshake_ack
from .ipc_client import DEFAULT_OUTBOUND_LIMIT, IpcClient, OutboundQueue
from .plans import PlanState


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

    def __init__(self, engine, config, queue=None):
        self._engine = engine
        self._config = config
        self._queue = queue if queue is not None else CommandQueue()
        self._pump_id = None
        self._state = "standby"
        self._events = []
        self._client = None
        self._handshake_generation = 0
        self._refusal_reason = ""
        self._dispatcher = CommandDispatcher(
            self._send_command_result, lambda: self._handshake_generation
        )
        self._plans = PlanState(config.swell_id, SCHEDULING_RESOLUTION_MS)
        self._dispatcher.set_handler("apply_plan", self._on_apply_plan)
        self._dispatcher.set_handler("start", self._on_start)

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
            )
            self._client.connect()
            # The handshake is the first outbound message (WM-0203); send is
            # a non-blocking enqueue, so on_init stays cheap on the engine
            # thread. A failed connect leaves it unsent — the state stays
            # `running` (handshake pending) and WM-0212's reconnection owns
            # the retry.
            if self._client.is_connected:
                self._client.send_message(build_handshake(self.capability_report()))
        self._state = "running"
        self._arm_pump()
        return self._state

    def on_finish(self):
        """Entry from the engine entry script's `onFinish()` (no args).

        Must be synchronous: the engine's finalise does not wait for async
        teardown (§8.3 #5), so no async work is armed here — the IPC client
        is closed inline (bounded join, never a detached wait) and the pump
        is simply cancelled.
        """
        if self._client is not None:
            self._client.close()
            self._client = None
        if self._pump_id is not None:
            self._engine.cancel(self._pump_id)
            self._pump_id = None
        self._state = "stopped"
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

    def _on_pump(self):
        try:
            self.drain_command_queue()
            # Batch plan execution rides the same pump tick: due batches
            # become one addBots call each, on the scripting thread (S1).
            self.execute_due_batches()
        except Exception:
            # S3: a script exception is printed and swallowed by the engine;
            # the bridge state machine must not depend on a callback having
            # succeeded, so log and keep the loop alive.
            traceback.print_exc()
        finally:
            # `ready` still has a live protocol session; `refused` has none
            # (the client is torn down and no traffic can flow), so the pump
            # ends there — the engine itself keeps running untouched.
            if self._state in ("running", "ready"):
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
        self._handshake_generation = generation
        self._state = "ready"

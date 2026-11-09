"""KBEngine Bridge bootstrap: the engine-side shell loaded by `onInit`.

WM-0201. The bootstrap owns:

  * the state machine (standby -> running -> stopped),
  * the inbound command queue (IPC threads only enqueue; see command_queue),
  * the engine-callback pump that drains the queue on the scripting thread,
  * the injected configuration (Supervisor passes IPC address, token, Swell
    ID, Backend ID — kbengine.md lifecycle step 3).

It performs no IPC itself and sends no handshake: the local IPC client is
WM-0202 and the handshake is WM-0203. Until an IPC address is injected the
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


class BootstrapConfig(object):
    """Supervisor-injected configuration for one Bridge instance.

    The injection channel is environment variables on the bot child process
    (WM-0205 may extend this to an explicit config file or CLI arguments).
    """

    ENV_IPC = "WAVEMAKER_BRIDGE_IPC"
    ENV_TOKEN = "WAVEMAKER_BRIDGE_TOKEN"
    ENV_SWELL_ID = "WAVEMAKER_SWELL_ID"
    ENV_BACKEND_ID = "WAVEMAKER_BACKEND_ID"

    def __init__(self, ipc_address="", token="", swell_id="", backend_id=""):
        self.ipc_address = ipc_address
        self.token = token
        self.swell_id = swell_id
        self.backend_id = backend_id

    @classmethod
    def from_env(cls, environ=None):
        env = os.environ if environ is None else environ
        return cls(
            ipc_address=env.get(cls.ENV_IPC, ""),
            token=env.get(cls.ENV_TOKEN, ""),
            swell_id=env.get(cls.ENV_SWELL_ID, ""),
            backend_id=env.get(cls.ENV_BACKEND_ID, ""),
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

    @property
    def state(self):
        return self._state

    @property
    def config(self):
        return self._config

    @property
    def queue(self):
        return self._queue

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
            "capabilities": CAPABILITIES,
            "scheduling_resolution_ms": SCHEDULING_RESOLUTION_MS,
            "arrival_batch_ms": ARRIVAL_BATCH_MS,
        }

    # -- engine entry points -------------------------------------------------

    def on_init(self, arg0=0):
        """Entry from the engine entry script's `onInit(arg0: int)`.

        The engine calls this exactly once per process (S6) with a fixed int
        argument 0 (audit §5); a second call is guarded. Throwing here aborts
        engine startup, so this is synchronous and non-throwing by design.
        """
        if self._state != "standby":
            # Already running or finished: no double init (S6).
            return self._state
        self._state = "running"
        self._arm_pump()
        return self._state

    def on_finish(self):
        """Entry from the engine entry script's `onFinish()` (no args).

        Must be synchronous: the engine's finalise does not wait for async
        teardown (§8.3 #5), so no async work is armed here — the pump is
        simply cancelled and the state transitions to stopped.
        """
        if self._pump_id is not None:
            self._engine.cancel(self._pump_id)
            self._pump_id = None
        self._state = "stopped"
        return self._state

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
        except Exception:
            # S3: a script exception is printed and swallowed by the engine;
            # the bridge state machine must not depend on a callback having
            # succeeded, so log and keep the loop alive.
            traceback.print_exc()
        finally:
            if self._state == "running":
                self._arm_pump()

    def drain_command_queue(self):
        """Consume every queued command on the scripting thread. Returns the
        number of commands processed (FIFO order)."""
        commands = self._queue.drain()
        for command in commands:
            self._handle_command(command)
        return len(commands)

    def _handle_command(self, command):
        # WM-0206+ dispatches real commands (ApplyPlan / Start / StopAdmission
        # / Drain / Shutdown) and routes results to the IPC sender. The
        # bootstrap only records, keeping the state machine honest until that
        # layer lands.
        self._events.append(command)

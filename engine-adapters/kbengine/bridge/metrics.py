"""Bridge queue and callback lag metrics (WM-0215, docs/bridge.md
§Metrics Model process level: "IPC queue depth" and
"event loop / callback lag").

CallbackLagMeter measures how late the engine fired the pump callback
beyond its requested delay. The pump arms a single-shot callback each tick
(audit §3/D4: the 100 ms gameTick quantum means the engine may fire late —
quantized up, never early by design); the excess over the requested delay
is the callback lag, and surfacing it makes the D4 quantization visible to
the Supervisor instead of silently absorbed. Fires before the due instant
are not lag — clamped at zero.

Queue depths and dropped counters are read from the existing bounded
structures (CommandQueue, IpcClient stats) — this module only owns the lag
bookkeeping; the bootstrap shapes the heartbeat block.
"""

import time


class CallbackLagMeter(object):
    """Callback-lag bookkeeping. Scripting-thread only (S1: the pump is
    the sole caller)."""

    def __init__(self, interval_s, clock=None):
        self._interval_s = interval_s
        self._clock = time.monotonic if clock is None else clock
        self._due_at = None
        self._last_s = 0.0
        self._max_s = 0.0

    def armed(self):
        """Record the due instant of the just-armed callback."""
        self._due_at = self._clock() + self._interval_s

    def fired(self):
        """Lag of this fire against the armed due instant (None when the
        pump fired without an armed record — defensive; the pump always
        arms before firing)."""
        if self._due_at is None:
            return None
        lag = max(0.0, self._clock() - self._due_at)
        self._due_at = None
        self._last_s = lag
        if lag > self._max_s:
            self._max_s = lag
        return lag

    def snapshot_ms(self):
        """`{callback_lag_ms, callback_lag_max_ms}` for the heartbeat.
        The max covers the window since the previous snapshot and resets
        here — every snapshot starts a new max window."""
        snapshot = {
            "callback_lag_ms": self._last_s * 1000.0,
            "callback_lag_max_ms": self._max_s * 1000.0,
        }
        self._max_s = 0.0
        return snapshot

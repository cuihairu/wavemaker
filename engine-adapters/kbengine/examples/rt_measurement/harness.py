"""Runtime measurement entry module (WM-0010, RT1/RT4).

Installs like the production kbemain (copy over the bots entry script) and
runs the production Bridge inside the real KBEngine bots process, with
measurement-only wrappers:

  RT1  callback scheduling granularity — the interval between consecutive
       pump fires, where each fire is the engine executing a KBEngine
       callback armed for +100 ms (the audit §3 quantum). The interval
       distribution is the granularity the engine actually delivers.
  RT4  Bridge queue latency — enqueue (IPC receive thread) to drain
       (scripting-thread pump) handoff time per command, measured on a
       MeasuredQueue injected through the production constructor.

Nothing here changes production behaviour: the wrappers observe the same
objects the production code uses. The report is written at engine finish
to $WAVEMAKER_RT_REPORT (default: /tmp/wm-rt-report.json).

Usage (run_rt.sh does this): copy this file over the bots entry script
(kbemain.py) next to the installed bridge/ package, then start bots with
WAVEMAKER_BRIDGE_IPC and WAVEMAKER_RT_REPORT set.
"""

import json
import os
import sys
import threading
import time

sys.path.insert(
    0, os.path.abspath(os.path.join(os.path.dirname(__file__))))

from bridge.bootstrap import Bootstrap, BootstrapConfig  # noqa: E402
from bridge.command_queue import CommandQueue  # noqa: E402
from bridge.engine_port import EnginePort  # noqa: E402

REPORT_PATH = os.environ.get("WAVEMAKER_RT_REPORT", "/tmp/wm-rt-report.json")


class MeasuredQueue(CommandQueue):
    """CommandQueue that timestamps the IPC->scripting handoff per command.

    enqueue runs on the IPC receive thread, drain on the scripting-thread
    pump — the delta is exactly the RT4 queue latency (including how long
    the command waited for its pump tick)."""

    def __init__(self):
        super().__init__()
        self._stamp_lock = threading.Lock()
        self._enqueued_at = {}
        self.latencies_ms = []

    def enqueue(self, command):
        with self._stamp_lock:
            self._enqueued_at[id(command)] = time.monotonic()
        return super().enqueue(command)

    def drain(self):
        commands = super().drain()
        now = time.monotonic()
        with self._stamp_lock:
            for command in commands:
                queued_at = self._enqueued_at.pop(id(command), None)
                if queued_at is not None:
                    self.latencies_ms.append((now - queued_at) * 1000.0)
        return commands


class FireRecorder(object):
    """Timestamps consecutive pump fires; the deltas are the RT1 samples.

    The wrap also owns the report write: the engine never reaches the
    script's onFinish on SIGTERM (verified on this build), so the report
    is written deterministically from the pump tick that observes the
    session teardown — the driver's shutdown command lands as the last
    processed tick, on the scripting thread, before any TERM can arrive."""

    def __init__(self):
        self._lock = threading.Lock()
        self._last = None
        self.intervals_ms = []
        self.report_written = False

    def wrap(self, bridge):
        original_pump = bridge._on_pump

        def measured_pump():
            now = time.monotonic()
            with self._lock:
                if self._last is not None:
                    self.intervals_ms.append((now - self._last) * 1000.0)
                self._last = now
            original_pump()
            if not self.report_written and \
                    bridge.state in ("stopped", "refused"):
                self.report_written = True
                write_report(bridge)

        # The pump re-arms with functools.partial(self._on_pump) on every
        # tick, so the instance attribute is re-read each cycle and this
        # replacement takes over from the second tick onward.
        bridge._on_pump = measured_pump


def percentiles(samples):
    if not samples:
        return None
    ordered = sorted(samples)

    def pct(fraction):
        index = min(len(ordered) - 1, int(round(fraction * (len(ordered) - 1))))
        return round(ordered[index], 3)

    return {
        "n": len(ordered),
        "min_ms": round(ordered[0], 3),
        "median_ms": pct(0.5),
        "p95_ms": pct(0.95),
        "max_ms": round(ordered[-1], 3),
    }


_measured_queue = None
_fire_recorder = None
_bridge = None


def write_report(bridge):
    """Dump the collected RT1/RT4 distributions to the report path."""
    report = {
        "backend_id": bridge._config.backend_id,
        "pump_interval_s": bridge.PUMP_INTERVAL_S,
        "rt1_fire_intervals_ms": percentiles(_fire_recorder.intervals_ms),
        "rt4_queue_latency_ms": percentiles(_measured_queue.latencies_ms),
    }
    try:
        with open(REPORT_PATH, "w") as handle:
            json.dump(report, handle, indent=1)
            handle.write("\n")
    except OSError as error:
        print("rt report write failed: %s" % error, file=sys.stderr)
    return report


def onInit(arg0):
    """Engine entry (audit §5): production bootstrap with the measured
    queue injected and the pump wrapped."""
    global _measured_queue, _fire_recorder, _bridge
    _measured_queue = MeasuredQueue()
    _fire_recorder = FireRecorder()
    config = BootstrapConfig.from_env()
    _bridge = Bootstrap(EnginePort.default(), config, queue=_measured_queue)
    _fire_recorder.wrap(_bridge)
    return _bridge.on_init(arg0)


def onFinish():
    """Graceful-exit path (SIGTERM never reaches here on this build — the
    teardown hook in the pump owns the report then)."""
    if _bridge is not None and not _fire_recorder.report_written:
        _fire_recorder.report_written = True
        write_report(_bridge)
        return _bridge.on_finish()
    return None

"""Unit tests for the WM-0215 bridge metrics (queue depth + callback lag).

Run from the adapter root:  cd engine-adapters/kbengine && python3 -m pytest tests
Pure logic + a fake clock — no sockets, no engine.
"""

from bridge.bootstrap import Bootstrap, BootstrapConfig
from bridge.engine_port import EnginePort, FakeEngine
from bridge.metrics import CallbackLagMeter


class FakeClock(object):
    def __init__(self):
        self.now = 0.0

    def __call__(self):
        return self.now

    def advance(self, seconds):
        self.now += seconds


def test_lag_is_zero_for_an_immediate_fire():
    clock = FakeClock()
    meter = CallbackLagMeter(0.1, clock=clock)
    meter.armed()
    assert meter.fired() == 0.0, "fired exactly at due -> no lag"
    snapshot = meter.snapshot_ms()
    assert snapshot["callback_lag_ms"] == 0.0
    assert snapshot["callback_lag_max_ms"] == 0.0


def test_lag_measures_the_excess_beyond_the_armed_delay():
    clock = FakeClock()
    meter = CallbackLagMeter(0.1, clock=clock)
    meter.armed()
    clock.advance(0.05)  # early
    clock.advance(0.07)  # fires 20 ms past the due instant
    lag = meter.fired()
    assert abs(lag - 0.02) < 1e-9, "lag = fire time - due time"
    assert abs(meter.snapshot_ms()["callback_lag_ms"] - 20.0) < 1e-6


def test_early_fire_is_not_lag():
    clock = FakeClock()
    meter = CallbackLagMeter(0.1, clock=clock)
    meter.armed()
    clock.advance(0.03)  # 70 ms early
    assert meter.fired() == 0.0, "clamped at zero"


def test_fired_without_armed_record_is_none():
    meter = CallbackLagMeter(0.1, clock=FakeClock())
    assert meter.fired() is None
    assert meter.snapshot_ms() == {"callback_lag_ms": 0.0,
                                   "callback_lag_max_ms": 0.0}


def test_max_lag_accumulates_until_the_snapshot_resets_it():
    clock = FakeClock()
    meter = CallbackLagMeter(0.1, clock=clock)
    meter.armed()
    clock.advance(0.15)
    meter.fired()  # 50 ms late
    meter.armed()
    clock.advance(0.12)
    meter.fired()  # 20 ms late
    snapshot = meter.snapshot_ms()
    assert abs(snapshot["callback_lag_max_ms"] - 50.0) < 1e-6
    assert abs(snapshot["callback_lag_ms"] - 20.0) < 1e-6, "last fire"
    meter.armed()
    clock.advance(0.11)
    meter.fired()  # 10 ms late, new max window
    assert abs(meter.snapshot_ms()["callback_lag_max_ms"] - 10.0) < 1e-6


def test_bootstrap_metrics_report_queue_depth_and_dropped():
    fe = FakeEngine()
    b = Bootstrap(EnginePort(fe), BootstrapConfig())
    assert b.bridge_metrics() == {
        "callback_lag_ms": 0.0,       # nothing fired yet
        "callback_lag_max_ms": 0.0,
        "queue_depth": 0,
        "queue_dropped": 0,
    }
    b.queue.enqueue({"kind": "command"})  # arrives mid-tick, not pumped yet
    b.queue.enqueue({"kind": "command"})
    assert b.bridge_metrics()["queue_depth"] == 2
    # The pump drains the queue before the heartbeat rides the same tick,
    # so a drained queue reports depth 0 — arrivals only show between the
    # drain and the snapshot inside one tick.
    b.on_init(0)
    fe.fire(b.pump_id)
    b.on_finish()
    assert b.bridge_metrics()["queue_depth"] == 0


def test_pump_tick_records_the_callback_lag():
    fe = FakeEngine()
    b = Bootstrap(EnginePort(fe), BootstrapConfig())
    b.on_init(0)
    fe.fire(b.pump_id)  # immediate fake fire -> no lag
    metrics = b.bridge_metrics()
    assert metrics["callback_lag_ms"] == 0.0
    assert metrics["callback_lag_max_ms"] == 0.0
    b.on_finish()

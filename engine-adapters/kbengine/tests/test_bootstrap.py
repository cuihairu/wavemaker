"""Unit tests for the WM-0201 KBEngine Bridge bootstrap.

Run from the adapter root:  cd engine-adapters/kbengine && python3 -m pytest tests
Uses the FakeEngine port (no KBEngine runtime needed); runtime conformance
against the real engine is WM-0216 / RT1.
"""

import threading

from bridge.bootstrap import Bootstrap, BootstrapConfig
from bridge.capability import (
    ARRIVAL_BATCH_MS,
    CAPABILITIES,
    PROTOCOL_VERSION,
    SCHEDULING_RESOLUTION_MS,
)
from bridge.command_queue import CommandQueue
from bridge.engine_port import EnginePort, FakeEngine


def make_bootstrap(**cfg):
    """Returns (fake_engine, bootstrap): the bootstrap drives the engine
    through the production port (EnginePort) wrapping the FakeEngine."""
    config = BootstrapConfig(
        ipc_address=cfg.get("ipc", ""),
        token=cfg.get("token", ""),
        swell_id=cfg.get("swell", ""),
        backend_id=cfg.get("backend", ""),
    )
    fe = FakeEngine()
    return fe, Bootstrap(EnginePort(fe), config)


def test_on_init_takes_one_arg_and_arms_pump():
    engine, b = make_bootstrap()
    b.on_init(0)
    assert b.state == "running"
    # The pump is a single 100 ms callback.
    assert len(engine.scheduled) == 1
    (delay, _fn), = [v for v in engine.scheduled.values()]
    assert abs(delay - Bootstrap.PUMP_INTERVAL_S) < 1e-9


def test_pump_drains_queue_fifo_and_rearms():
    engine, b = make_bootstrap()
    b.on_init(0)
    b.queue.enqueue("a")
    b.queue.enqueue("b")
    pump_id = b.pump_id
    engine.fire(pump_id)  # single-shot: fires once, then re-arms
    assert b._events == ["a", "b"]
    assert b.state == "running"
    assert len(engine.scheduled) == 1, "pump re-armed after drain"


def test_enqueue_from_any_thread_never_calls_engine():
    # §8.3 #1: the IPC thread only enqueues; no script call happens there.
    engine, b = make_bootstrap()
    b.on_init(0)
    before = dict(engine.scheduled)
    t = threading.Thread(target=b.queue.enqueue, args=("from-ipc",))
    t.start()
    t.join()
    assert engine.scheduled == before, "enqueue path must not touch the engine"
    assert b.queue.pending() == 1


def test_concurrent_producers_execute_exactly_once_on_pump_thread():
    """WM-0009 runtime evidence: commands arriving concurrently from many
    IPC-side threads execute exactly once, in the single pump (scripting
    thread), and the pump survives the burst with its single-shot re-arm
    discipline intact (§8.3 #1; S1 makes cross-thread script calls UB)."""
    from collections import Counter

    producers = 8
    per_producer = 50
    engine, b = make_bootstrap()
    b.on_init(0)
    errors = []

    def produce(tag):
        try:
            for i in range(per_producer):
                assert b.queue.enqueue((tag, i)), "queue is unbounded here"
        except Exception as exc:  # pragma: no cover - diagnostics only
            errors.append(exc)

    threads = [
        threading.Thread(target=produce, args=(tag,)) for tag in range(producers)
    ]
    for t in threads:
        t.start()

    # Drive the pump exactly the way the engine does: single-shot fire, the
    # handler drains and re-arms with a fresh id. Nothing else consumes.
    total = 0
    deadline = 2000
    while total < producers * per_producer and deadline:
        deadline -= 1
        engine.fire(b.pump_id)
        total = len(b._events)
    for t in threads:
        t.join()

    assert not errors, "producers never fail"
    assert total == producers * per_producer, "pump drained everything"
    counts = Counter(b._events)
    assert set(counts) == {
        (tag, i) for tag in range(producers) for i in range(per_producer)
    }, "every command observed"
    assert all(c == 1 for c in counts.values()), "exactly once, no duplicates"
    assert len(engine.scheduled) == 1, "exactly one live pump callback"
    assert engine.cancelled == [], "nothing was cancelled mid-run"


def test_callback_exception_does_not_stop_pump():
    # S3: script exceptions are printed and swallowed; the state machine and
    # the pump must survive a failing handler.
    class Boom(object):
        def __init__(self, b):
            self.b = b
            self.calls = 0

        def __call__(self):
            self.calls += 1
            raise RuntimeError("boom")

    engine, b = make_bootstrap()
    b._handle_command = Boom(b)  # make every command raise
    b.on_init(0)
    b.queue.enqueue("x")
    engine.fire(b.pump_id)
    assert b.state == "running"
    assert len(engine.scheduled) == 1, "pump re-armed even after a failure"


def test_on_finish_is_synchronous_and_cancels_pump():
    # §8.3 #5: no async teardown, no re-arm after finish.
    engine, b = make_bootstrap()
    b.on_init(0)
    pump_id = b.pump_id
    b.on_finish()
    assert b.state == "stopped"
    assert pump_id in engine.cancelled, "pump handle cancelled"
    assert len(engine.scheduled) == 0, "no callback survives finish"
    # A late pump fire (already cancelled) must not re-arm.
    assert pump_id not in engine.scheduled


def test_on_init_is_guarded_against_second_call():
    # S6: onInit is once per process; a second call must not double-arm.
    engine, b = make_bootstrap()
    b.on_init(0)
    b.on_init(0)
    assert len(engine.scheduled) == 1, "exactly one pump callback"


def test_capability_catalog_is_honest():
    assert PROTOCOL_VERSION == 1
    sched = CAPABILITIES["scheduling"]
    assert sched["exact_arrival"] == "degraded", "S8 batches arrivals"
    assert sched["batch_arrival"] == "supported"
    assert sched["arrival_batch_ms"] == ARRIVAL_BATCH_MS == 1000
    assert sched["resolution_ms"] == SCHEDULING_RESOLUTION_MS == 100
    # No unverified capability claims (docs/kbengine.md).
    assert CAPABILITIES["bots"]["destroy_individual"] == "unknown"
    assert CAPABILITIES["bots"]["inspect_individual"] == "unknown"
    assert CAPABILITIES["observability"]["message_latency"] == "unsupported"
    assert CAPABILITIES["observability"]["connected_count"] == "unknown"


def test_config_from_env_injection():
    import os

    env = {
        "WAVEMAKER_BRIDGE_IPC": "/tmp/wm.sock",
        "WAVEMAKER_BRIDGE_TOKEN": "tok",
        "WAVEMAKER_SWELL_ID": "sw-1",
        "WAVEMAKER_BACKEND_ID": "be-1",
        "WAVEMAKER_BACKEND_GENERATION": "12",
    }
    cfg = BootstrapConfig.from_env(env)
    assert cfg.ipc_address == "/tmp/wm.sock"
    assert cfg.token == "tok"
    assert cfg.swell_id == "sw-1"
    assert cfg.backend_id == "be-1"
    assert cfg.backend_generation == 12
    assert cfg.has_ipc

    empty = BootstrapConfig.from_env({})
    assert not empty.has_ipc
    assert empty.ipc_address == ""
    assert empty.backend_generation == 0, "missing generation is unset"


def test_config_generation_malformed_values_are_unset():
    # onInit must never raise on bad injection (audit §5), so any value
    # that is not a positive decimal becomes 0.
    for raw, expected in [
        ("not-a-number", 0),
        ("", 0),
        ("  7  ", 7),  # surrounding whitespace tolerated
        ("-3", 0),
        ("0", 0),
        (None, 0),
        (12, 12),  # already an int passes through
    ]:
        cfg = BootstrapConfig(backend_generation=BootstrapConfig._parse_generation(raw))
        assert cfg.backend_generation == expected, f"{raw!r} -> {expected}"


def test_capability_report_is_handshake_shaped():
    engine, b = make_bootstrap(backend="be-9")
    report = b.capability_report()
    assert report["protocol_version"] == PROTOCOL_VERSION
    assert report["engine_name"] == "kbengine"
    assert report["adapter_name"] == "wavemaker-kbengine-bridge"
    assert report["backend_id"] == "be-9"
    assert report["backend_generation"] == 0, "unset by default"
    assert report["capabilities"] is CAPABILITIES


def test_command_queue_bounded_overflow():
    q = CommandQueue(max_size=2)
    assert q.enqueue("a")
    assert q.enqueue("b")
    assert not q.enqueue("c"), "overflow drops the newest"
    assert q.overflow_count() == 1
    assert q.drain() == ["a", "b"]
    assert q.pending() == 0

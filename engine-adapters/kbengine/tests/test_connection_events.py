"""Unit tests for WM-0209 connection event collection.

Run from the adapter root:  cd engine-adapters/kbengine && python3 -m pytest tests
Pure logic — FakeBots snapshots drive the diff; a fake clock timestamps.
"""

from bridge.bootstrap import Bootstrap, BootstrapConfig
from bridge.connection_events import (
    EVENT_BOT_CREATED,
    EVENT_BOT_DISCONNECTED,
    ConnectionCollector,
)
from bridge.engine_port import EnginePort, FakeBots, FakeEngine


def test_first_poll_is_baseline_not_a_flood():
    bots = FakeBots()
    bots.add(1)
    bots.add(2)
    collector = ConnectionCollector()
    assert collector.poll(bots) == [], "pre-existing bots are not reported"
    assert collector.take_events() == []


def test_new_client_id_is_bot_created_with_appid_bot_id():
    collector = ConnectionCollector()
    bots = FakeBots()
    collector.poll(bots)
    bots.add(42)  # appID — stable bot id for the client's lifetime
    events = collector.poll(bots)
    assert len(events) == 1
    assert events[0]["event_type"] == EVENT_BOT_CREATED
    assert events[0]["bot_id"] == 42
    assert isinstance(events[0]["source_timestamp"], float)
    assert collector.take_events() == events, "buffered for the event layer"


def test_removed_client_id_is_bot_disconnected():
    collector = ConnectionCollector()
    bots = FakeBots()
    bots.add(7)
    collector.poll(bots)
    bots.remove(7)  # delClient: kick/close/destroy all land here
    events = collector.poll(bots)
    assert len(events) == 1
    assert events[0]["event_type"] == EVENT_BOT_DISCONNECTED
    assert events[0]["bot_id"] == 7


def test_simultaneous_change_reports_created_then_disconnected_sorted():
    collector = ConnectionCollector()
    bots = FakeBots()
    bots.add(3)
    bots.add(1)
    collector.poll(bots)
    bots.remove(3)
    bots.remove(1)
    bots.add(9)
    bots.add(5)
    events = collector.poll(bots)
    assert [(e["event_type"], e["bot_id"]) for e in events] == [
        (EVENT_BOT_CREATED, 5),
        (EVENT_BOT_CREATED, 9),
        (EVENT_BOT_DISCONNECTED, 1),
        (EVENT_BOT_DISCONNECTED, 3),
    ], "created first (ascending), then removed (ascending)"


def test_none_mapping_keeps_the_baseline():
    collector = ConnectionCollector()
    bots = FakeBots()
    bots.add(4)
    collector.poll(bots)
    assert collector.poll(None) == [], "uncollectable tick is skipped"
    bots.add(6)
    events = collector.poll(bots)
    assert [e["bot_id"] for e in events] == [6], "baseline preserved"


def test_overflow_drops_newest_and_counts():
    collector = ConnectionCollector(clock=lambda: 0.0, max_pending=2)
    bots = FakeBots()
    collector.poll(bots)
    for i in range(5):
        bots.add(i)
        collector.poll(bots)
    assert collector.dropped_count() == 3
    events = collector.take_events()
    assert [e["bot_id"] for e in events] == [0, 1], "oldest kept"
    assert collector.take_events() == []


def test_bootstrap_polls_connection_events_on_pump():
    fe = FakeEngine()
    b = Bootstrap(EnginePort(fe), BootstrapConfig())
    b.on_init(0)
    fe.fire(b.pump_id)  # baseline tick
    fe.bots.add(77)
    fe.fire(b.pump_id)  # diff tick
    # WM-0213: the pump drains the raw records into the event stream, so
    # they surface as sequenced envelopes, not raw records. This standby
    # config has no IPC client, so the tick's batch was undeliverable —
    # the envelope stays in the replay window (cursor resume is the
    # durable path).
    assert b.take_connection_events() == []
    assert b.event_stream.last_sequence == 1
    b.event_stream.request_replay(0)
    replay = b.event_stream.build_replay_if_requested()
    assert replay["events"][0]["event_type"] == EVENT_BOT_CREATED
    assert replay["events"][0]["bot_id"] == 77
    b.on_finish()

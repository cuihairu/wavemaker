"""Unit tests for the WM-0213 event stream (sequencing, cursor replay,
dedup-stable envelopes).

Run from the adapter root:  cd engine-adapters/kbengine && python3 -m pytest tests
Pure logic — no sockets, no engine; transport (sending the batch) is the
pump's concern and is covered by the wire tests.
"""

from bridge.event_stream import SCHEMA_VERSION, EventStream


def raw(bot_id=1, event_type="bot_created", ts=1.5):
    return {"event_type": event_type, "bot_id": bot_id,
            "source_timestamp": ts}


def make_stream(shard="shard-3", max_window=8, clock=lambda: 9.0):
    return EventStream("kb-1", "sw-1", lambda: shard,
                       max_window=max_window, clock=clock)


ENVELOPE_FIELDS = (
    "event_id", "backend_id", "swell_id", "shard_id", "bot_id",
    "sequence", "source_timestamp", "observed_timestamp", "event_type",
    "payload", "schema_version",
)


def test_assign_builds_the_full_docs_bridge_md_envelope():
    stream = make_stream()
    events = stream.assign([raw(bot_id=7, ts=1.25)])
    assert len(events) == 1
    envelope = events[0]
    for field in ENVELOPE_FIELDS:
        assert field in envelope, f"envelope lacks {field}"
    assert envelope["event_id"] == "kb-1:1"
    assert envelope["backend_id"] == "kb-1"
    assert envelope["swell_id"] == "sw-1"
    assert envelope["shard_id"] == "shard-3"
    assert envelope["bot_id"] == 7
    assert envelope["sequence"] == 1
    assert envelope["source_timestamp"] == 1.25
    assert envelope["observed_timestamp"] == 9.0
    assert envelope["event_type"] == "bot_created"
    assert envelope["payload"] == {}
    assert envelope["schema_version"] == SCHEMA_VERSION


def test_sequences_are_strictly_monotonic_and_event_ids_deterministic():
    stream = make_stream()
    first = stream.assign([raw(), raw(bot_id=2), raw(bot_id=3)])
    assert [e["sequence"] for e in first] == [1, 2, 3]
    assert [e["event_id"] for e in first] == ["kb-1:1", "kb-1:2", "kb-1:3"]
    second = stream.assign([raw(bot_id=4)])
    assert second[0]["sequence"] == 4, "assignment never reuses a sequence"
    assert stream.last_sequence == 4


def test_shard_attribution_follows_the_plan_live():
    holder = {"shard": ""}
    stream = EventStream("kb-1", "sw-1", lambda: holder["shard"],
                         clock=lambda: 0.0)
    assert stream.assign([raw()])[0]["shard_id"] == ""
    holder["shard"] = "shard-5"  # plan accepted between ticks
    assert stream.assign([raw(bot_id=2)])[0]["shard_id"] == "shard-5"


def test_build_batch_needs_no_outbox_and_carries_the_max_cursor():
    stream = make_stream()
    assert stream.build_batch([]) is None
    events = stream.assign([raw(), raw(bot_id=2)])
    batch = stream.build_batch(events)
    assert batch["kind"] == "events"
    assert batch["backend_id"] == "kb-1"
    assert [e["event_id"] for e in batch["events"]] == ["kb-1:1", "kb-1:2"]
    assert batch["cursor"] == 2, "cursor is the batch's highest sequence"


def test_replay_after_cursor_resends_identical_envelopes():
    stream = make_stream()
    first = stream.assign([raw(), raw(bot_id=2)])
    later = stream.assign([raw(bot_id=3)])

    stream.request_replay(1)  # Supervisor durably processed sequence 1
    batch = stream.build_replay_if_requested()
    assert stream.build_replay_if_requested() is None, "one shot"
    assert batch["kind"] == "events"
    ids = [e["event_id"] for e in batch["events"]]
    assert ids == ["kb-1:2", "kb-1:3"], "only events after the cursor"
    assert batch["cursor"] == 3
    assert "gap" not in batch and "oldest_available" not in batch
    # Dedup stability: the replayed envelopes are the exact stored ones —
    # never re-sequenced, never mutated.
    assert batch["events"][0] is first[1]
    assert batch["events"][1] is later[0]


def test_replay_without_a_cursor_replays_the_whole_window():
    stream = make_stream()
    stream.assign([raw(), raw(bot_id=2)])
    stream.request_replay(0)
    batch = stream.build_replay_if_requested()
    assert [e["event_id"] for e in batch["events"]] == ["kb-1:1", "kb-1:2"]


def test_replay_reports_an_honest_gap_past_the_window():
    stream = make_stream(max_window=3)
    stream.assign([raw(bot_id=i) for i in range(1, 6)])  # seq 1..5, 1-2 evicted
    stream.request_replay(0)  # Supervisor never saw 1..2; they are gone
    batch = stream.build_replay_if_requested()
    ids = [e["event_id"] for e in batch["events"]]
    assert ids == ["kb-1:3", "kb-1:4", "kb-1:5"]
    assert batch["gap"] is True
    assert batch["oldest_available"] == 3, \
        "unrecoverable range is [1, 2] — never silently skipped"


def test_replay_on_an_empty_window_reports_loss_after_the_last_event():
    stream = make_stream()
    stream.assign([raw()])  # seq 1
    stream.request_replay(1)  # Supervisor is fully caught up
    batch = stream.build_replay_if_requested()
    assert batch["events"] == []
    assert batch["cursor"] == 1
    assert "gap" not in batch

    stream.assign([raw(bot_id=2)])  # seq 2, then the window is emptied
    stream._window.clear()  # simulate a flushed/expired window
    stream.request_replay(0)
    batch = stream.build_replay_if_requested()
    assert batch["events"] == []
    assert batch["cursor"] == 2
    assert batch["gap"] is True
    assert batch["oldest_available"] == 3, \
        "nothing retained; the next event to come would be 3"


def test_window_overflow_drops_the_oldest_and_counts_it():
    stream = make_stream(max_window=2)
    stream.assign([raw(bot_id=1), raw(bot_id=2)])
    assert stream.dropped_count() == 0
    stream.assign([raw(bot_id=3)])  # seq 1 falls off the deque
    assert stream.dropped_count() == 1
    stream.request_replay(0)
    batch = stream.build_replay_if_requested()
    ids = [e["event_id"] for e in batch["events"]]
    assert ids == ["kb-1:2", "kb-1:3"]
    assert batch["gap"] is True and batch["oldest_available"] == 2


def test_empty_stream_replay_is_a_position_ack_not_a_gap():
    stream = make_stream()
    stream.request_replay(0)
    batch = stream.build_replay_if_requested()
    assert batch["events"] == []
    assert batch["cursor"] == 0
    assert "gap" not in batch and "oldest_available" not in batch

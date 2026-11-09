"""Unit tests for the WM-0207 ApplyPlan batch plan mapping.

Run from the adapter root:  cd engine-adapters/kbengine && python3 -m pytest tests
Pure logic — no sockets, no engine; a fake clock drives the batch schedule.
"""

from bridge.plans import (
    OK,
    PlanState,
    parse_plan_payload,
)


def plan(swell_id="sw-1", digest="sha256:abc", batches=None):
    return {
        "swell_id": swell_id,
        "plan_digest": digest,
        "batches": batches
        if batches is not None
        else [
            {"offset_ms": 0, "bot_count": 10},
            {"offset_ms": 300, "bot_count": 20},
        ],
    }


class FakeClock(object):
    def __init__(self):
        self.now = 0.0

    def __call__(self):
        return self.now

    def advance(self, seconds):
        self.now += seconds


def test_parse_rejects_malformed_payloads_with_stable_reasons():
    cases = [
        (None, "plan_not_an_object"),
        ([], "plan_not_an_object"),
        ({"plan_digest": "d", "batches": []}, "swell_id_missing"),
        ({"swell_id": "sw", "batches": []}, "plan_digest_missing"),
        ({"swell_id": "sw", "plan_digest": ""}, "plan_digest_missing"),
        ({"swell_id": "sw", "plan_digest": "d"}, "batches_missing"),
        ({"swell_id": "sw", "plan_digest": "d", "batches": []},
         "batches_missing"),
        ({"swell_id": "sw", "plan_digest": "d", "batches": ["x"]},
         "bad_batch"),
        ({"swell_id": "sw", "plan_digest": "d",
          "batches": [{"offset_ms": -1, "bot_count": 1}]}, "bad_batch"),
        ({"swell_id": "sw", "plan_digest": "d",
          "batches": [{"offset_ms": 0, "bot_count": -2}]}, "bad_batch"),
        ({"swell_id": "sw", "plan_digest": "d",
          "batches": [{"offset_ms": True, "bot_count": 1}]}, "bad_batch"),
        ({"swell_id": "sw", "plan_digest": "d",
          "batches": [{"offset_ms": 0.5, "bot_count": 1}]}, "bad_batch"),
    ]
    for payload, reason in cases:
        result = parse_plan_payload(payload, 100)
        assert isinstance(result, tuple), f"{payload!r} should be rejected"
        code, reason_code, detail = result
        assert code == "INVALID_ARGUMENT"
        assert reason_code == reason, f"{payload!r} -> {reason_code}"
        assert detail, "every rejection carries an explanation"


def test_parse_quantizes_and_coalesces_onto_resolution_buckets():
    parsed = parse_plan_payload(
        plan(batches=[
            {"offset_ms": 50, "bot_count": 5},
            {"offset_ms": 80, "bot_count": 7},
            {"offset_ms": 250, "bot_count": 9},
            {"offset_ms": 999, "bot_count": 1},
        ]),
        100,
    )
    # 50/80 coalesce into bucket 0; 250 -> 2; 999 -> 9.
    assert parsed["buckets"] == [(0, 12), (2, 9), (9, 1)]
    assert parsed["swell_id"] == "sw-1"
    assert parsed["plan_digest"] == "sha256:abc"


def test_apply_accepts_and_stores_the_frozen_plan():
    ps = PlanState("sw-1", 100)
    assert ps.state == "idle"
    assert ps.apply(plan()) == OK
    assert ps.state == "plan_accepted"
    assert ps.plan_digest == "sha256:abc"


def test_apply_is_idempotent_for_the_same_digest():
    ps = PlanState("sw-1", 100)
    assert ps.apply(plan()) == OK
    assert ps.apply(plan()) == OK, "replay of the accepted plan is OK"


def test_apply_rejects_swell_mismatch_and_conflicting_digests():
    ps = PlanState("sw-1", 100)
    code, reason, detail = ps.apply(plan(swell_id="other"))
    assert (code, reason) == ("INVALID_ARGUMENT", "swell_mismatch")
    assert "sw-1" in detail

    assert ps.apply(plan(digest="sha256:a")) == OK
    code, reason, detail = ps.apply(plan(digest="sha256:b"))
    assert (code, reason) == ("FAILED_PRECONDITION", "conflicting_plan")
    assert "sha256:a" in detail and "sha256:b" in detail


def test_apply_while_running_is_refused_not_silently_swapped():
    clock = FakeClock()
    ps = PlanState("sw-1", 100, clock=clock)
    ps.apply(plan())
    ps.start("op-1")
    code, reason, detail = ps.apply(plan(digest="sha256:new"))
    assert (code, reason) == ("FAILED_PRECONDITION", "conflicting_plan")
    assert ps.plan_digest == "sha256:abc", "running shard never replaced"


def test_start_before_apply_is_failed_precondition():
    ps = PlanState("sw-1", 100)
    code, reason, detail = ps.start("op-1")
    assert (code, reason) == ("FAILED_PRECONDITION", "plan_not_accepted")
    assert detail


def test_start_is_idempotent_for_the_same_operation_id():
    clock = FakeClock()
    ps = PlanState("sw-1", 100, clock=clock)
    ps.apply(plan())
    assert ps.start("op-1") == OK
    assert ps.state == "running"
    assert ps.start_operation_id == "op-1"
    assert ps.start("op-1") == OK, "same operation_id replays OK"
    code, reason, detail = ps.start("op-2")
    assert (code, reason) == ("FAILED_PRECONDITION", "already_running")


def test_due_batches_follow_the_quantized_schedule():
    clock = FakeClock()
    ps = PlanState("sw-1", 100, clock=clock)
    ps.apply(plan(batches=[
        {"offset_ms": 0, "bot_count": 10},
        {"offset_ms": 300, "bot_count": 20},
    ]))
    assert ps.due_batches() == [], "nothing due before start"
    ps.start("op-1")
    assert ps.due_batches() == [10], "bucket 0 fires at start"
    clock.advance(0.05)  # halfway into bucket 0 — nothing new
    assert ps.due_batches() == []
    clock.advance(0.05)  # bucket 1
    assert ps.due_batches() == []
    clock.advance(0.2)  # now in bucket 3: bucket 3 batch is due
    assert ps.due_batches() == [20]
    clock.advance(1.0)
    assert ps.due_batches() == [], "schedule exhausted"
    assert ps.due_batches() == []


def test_coalesced_same_bucket_batches_fire_as_one_addbots():
    ps = PlanState("sw-1", 100)
    ps.apply(plan(batches=[
        {"offset_ms": 10, "bot_count": 3},
        {"offset_ms": 60, "bot_count": 4},
    ]))
    ps.start("op-1")
    assert ps.due_batches() == [7], "same bucket sums into one addBots call"

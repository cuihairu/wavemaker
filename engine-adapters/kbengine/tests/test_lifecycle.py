"""Unit tests for WM-0211 Drain / Shutdown lifecycle.

Run from the adapter root:  cd engine-adapters/kbengine && python3 -m pytest tests
"""

from bridge.lifecycle import (
    STATE_ACTIVE,
    STATE_DRAINING,
    STATE_STOPPED,
    BackendLifecycle,
)


def test_drain_transitions_one_way_and_is_idempotent_per_operation_id():
    lc = BackendLifecycle()
    assert lc.state == STATE_ACTIVE
    assert lc.drain("drain-1") == ("OK", "", "")
    assert lc.state == STATE_DRAINING
    assert lc.drain_operation_id == "drain-1"
    assert lc.drain("drain-1") == ("OK", "", ""), "same operation_id replays"
    code, reason, detail = lc.drain("drain-2")
    assert (code, reason) == ("FAILED_PRECONDITION", "already_draining")


def test_shutdown_is_idempotent_without_a_key():
    lc = BackendLifecycle()
    assert lc.shutdown("down-1") == ("OK", "", "")
    assert lc.state == STATE_STOPPED
    assert lc.shutdown("down-2") == ("OK", "", ""), "idempotent, any id"


def test_drain_after_shutdown_is_already_stopped():
    lc = BackendLifecycle()
    lc.shutdown("down-1")
    code, reason, detail = lc.drain("drain-1")
    assert (code, reason) == ("FAILED_PRECONDITION", "already_stopped")
    assert detail

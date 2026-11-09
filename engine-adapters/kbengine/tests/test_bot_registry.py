"""Unit tests for WM-0208 bot count collection.

Run from the adapter root:  cd engine-adapters/kbengine && python3 -m pytest tests
Pure logic — a FakeBots object mirrors the PyBots contract (audit §2).
"""

import pytest

from bridge.bot_registry import collect_bot_snapshot
from bridge.bootstrap import Bootstrap, BootstrapConfig
from bridge.engine_port import EnginePort, FakeBots, FakeEngine


class DestroyedBot(object):
    isDestroyed = True


def test_snapshot_counts_live_bots_with_appid_keys():
    bots = FakeBots()
    bots.add(101)  # appIDs are int32 clientIDs, never entityIDs
    bots.add(202)
    bots.add(303)
    snap = collect_bot_snapshot(bots)
    assert snap["bot_count"] == 3
    assert snap["bot_ids"] == [101, 202, 303]
    assert snap["dead_ids"] == []


def test_snapshot_guards_isdestroyed_clients():
    bots = FakeBots()
    bots.add(1, DestroyedBot())
    bots.add(2)
    snap = collect_bot_snapshot(bots)
    assert snap["bot_count"] == 1
    assert snap["bot_ids"] == [2]
    assert snap["dead_ids"] == [1]


def test_snapshot_guards_vanished_between_keys_and_access():
    # §8.3 #2: keys() snapshot first, then per-id guarded access. A client
    # removed in the same tick's delClient pass between the two steps must
    # count as dead, never crash the pump.
    bots = FakeBots()
    bots.add(7)
    bots.add(8)

    class Vanishing(FakeBots):
        def __getitem__(self, client_id):
            if client_id == 7:
                raise KeyError(client_id)
            return FakeBots.__getitem__(self, client_id)

    vanishing = Vanishing()
    vanishing._clients = bots._clients
    snap = collect_bot_snapshot(vanishing)
    assert snap["bot_count"] == 1
    assert snap["bot_ids"] == [8]
    assert snap["dead_ids"] == [7]


def test_missing_isdestroyed_attribute_assumes_alive():
    # Runtime-verification pending (audit §2 open item): ClientObject's
    # Python-visible attribute list is not established, so the guard
    # degrades to "assume alive" — no invented facts.
    bots = FakeBots()
    bots.add(5, object())
    snap = collect_bot_snapshot(bots)
    assert snap["bot_count"] == 1
    assert snap["dead_ids"] == []


def test_none_bots_is_uncollectable_not_zero():
    # No bots mapping means "cannot collect", never "0 bots" — no fake
    # zeros (docs/kbengine.md no-unverified-claims).
    assert collect_bot_snapshot(None) is None


def test_bots_mapping_is_read_only():
    # pybots.cpp: mp_ass_subscript is NULL — Python cannot add/remove.
    bots = FakeBots()
    with pytest.raises(TypeError):
        bots[9] = object()


def test_bootstrap_bot_snapshot_wiring():
    fe = FakeEngine()
    fe.bots.add(11)
    fe.bots.add(12, DestroyedBot())
    b = Bootstrap(EnginePort(fe), BootstrapConfig())
    snap = b.bot_snapshot()
    assert snap["bot_count"] == 1
    assert snap["bot_ids"] == [11]
    assert snap["dead_ids"] == [12]


def test_bootstrap_without_bots_mapping_reports_none():
    class NoBotsEngine(FakeEngine):
        pass

    fe = NoBotsEngine()
    del fe.bots
    b = Bootstrap(EnginePort(fe), BootstrapConfig())
    assert b.bot_snapshot() is None

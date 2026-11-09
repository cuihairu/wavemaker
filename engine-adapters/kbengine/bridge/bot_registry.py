"""Bot count collection (WM-0208, audit §2 + §8.3 #2).

`KBEngine.bots` is a PyBots object, not a dict (audit §2):

  * keys are int32 clientIDs (appIDs) — never entityIDs or ordinals;
  * `keys()` returns an INCREF'd snapshot (pybots.cpp:89-150 builds a fresh
    list without re-entering Python), but the listed ClientObjects can be
    destroyed afterwards;
  * `bots[id]` reads the live map and raises KeyError for a missing id;
  * the mapping is read-only (mp_ass_subscript is NULL — Python cannot
    add or remove bots; addBots is the only injection point).

Collection discipline (§8.3 #2, following the `bots.cpp:286-290`
`isDestroyed` guard pattern): take the `keys()` snapshot first, then touch
each id guarded — a bot that vanished between snapshot and access counts
as dead, never crashes the pump. Runs on the scripting thread (S1: the
pump is the only caller, and bot add/remove happens on that same tick, so
no torn iteration is possible).

Honesty notes (docs/kbengine.md no-unverified-claims):

  * the live count comes from the guarded traversal, not bare `len()` —
    `len()` reads the live map and can briefly include clients already
    logically destroyed (removal happens in delClient on the same tick, so
    the discrepancy window is one tick at most);
  * `isDestroyed` presence on Python-visible ClientObjects is
    runtime-verification pending (audit §2 open item) — the guard uses
    `getattr(obj, "isDestroyed", False)` so a missing attribute degrades to
    "assume alive" instead of inventing a fact.
"""

# Snapshot keys (docs/kbengine.md event model uses these names for the
# count report; the wire push itself rides the event/metrics items).
SNAPSHOT_BOT_COUNT = "bot_count"
SNAPSHOT_BOT_IDS = "bot_ids"
SNAPSHOT_DEAD = "dead_ids"


def collect_bot_snapshot(bots):
    """Snapshot the PyBots mapping -> `{bot_count, bot_ids, dead_ids}`.

    `bots` is the live `KBEngine.bots` object (or None when the engine
    module exposes no bots mapping — e.g. unit environments; the caller
    reports that as uncollectable rather than inventing a zero).
    """
    if bots is None:
        return None
    ids = list(bots.keys())  # INCREF snapshot: safe against later removals
    alive = []
    dead = []
    for client_id in ids:
        try:
            obj = bots[client_id]
        except KeyError:
            # Vanished between the snapshot and this access (removed in the
            # same tick's delClient pass) — dead, not missing.
            dead.append(client_id)
            continue
        if getattr(obj, "isDestroyed", False):
            dead.append(client_id)
            continue
        alive.append(client_id)
    return {
        SNAPSHOT_BOT_COUNT: len(alive),
        SNAPSHOT_BOT_IDS: alive,
        SNAPSHOT_DEAD: dead,
    }

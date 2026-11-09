"""Connection and login event collection (WM-0209, audit §1-§2 facts).

What is statically collectable from the target KBEngine source (docs/spec/
kbengine-api-audit.md):

  * a ClientObject appearing in the `KBEngine.bots` mapping = the creation
    step of a login attempt (CreateAndLoginHandler -> bots.cpp:500-506);
  * a clientID vanishing from the mapping = delClient (bots.cpp:286-290,
    515-525) — the disconnect/destroy path. The engine does not distinguish
    kick vs clean close vs destroy at this layer, so all removals are
    reported as BotDisconnected (no invented distinctions);
  * the keys() INCREF snapshot makes the diff race-free on the scripting
    thread (S1; §8.3 #2 discipline — snapshot first, no per-item access
    needed for the diff).

What is NOT collected here, on purpose:

  * BotConnected / BotLoginSucceeded / BotLoginFailed need the per-client
    connection state (C_STATE_* in clientobject.cpp:414-455) or the
    ClientObjectBase event stream (EventData_onKicked,
    clientobjectbase.cpp:240-247) being exposed to Python — the audit marks
    the Python-visible ClientObject surface runtime-verification pending
    (capability catalog: login_success/connected_count/per_bot_events =
    unknown). A future collector plugs in beside this one once a real
    engine (WM-0216) establishes the API; nothing is faked here.

Event records are raw collection-layer output: `{event_type, bot_id,
source_timestamp}` with `bot_id` = the int32 appID (stable per client
object, lifecycle = the client's life in the bots process — bridge.md's
stable-id requirement). The full wire envelope (event_id, sequence,
backend/shell attribution, observed_timestamp, dedup) is WM-0213's job;
this layer only observes and buffers with a bounded overflow guard.
"""

import time

EVENT_BOT_CREATED = "BotCreated"
EVENT_BOT_DISCONNECTED = "BotDisconnected"

# Buffer bound: the consumer (WM-0213 event layer) is expected to drain
# every pump tick; the bound only guards a stalled consumer. Overflow drops
# the newest observation and counts it (same discipline as CommandQueue).
MAX_PENDING_EVENTS = 4096

# login-state collection is not implementable on statically-known APIs —
# recorded here so the capability catalog and the code agree.
LOGIN_STATE_COLLECTABLE = False


class ConnectionCollector(object):
    """Diffs consecutive `KBEngine.bots.keys()` snapshots into raw
    connection events. Scripting-thread only (S1: the pump is the sole
    caller, and bot add/remove happens on that same tick)."""

    def __init__(self, clock=None, max_pending=MAX_PENDING_EVENTS):
        self._clock = time.monotonic if clock is None else clock
        self._known = None
        self._pending = []
        self._max_pending = max_pending
        self._dropped = 0

    def poll(self, bots):
        """Observe the current bots mapping; returns the new raw events
        (also buffered for `take_events`). A None mapping (uncollectable,
        see bot_registry) is skipped, keeping the previous baseline."""
        if bots is None:
            return []
        snapshot = set(bots.keys())
        if self._known is None:
            # First observation is the baseline: bots created before the
            # collector started are not retro-actively reported.
            self._known = snapshot
            return []
        now = self._clock()
        created = sorted(snapshot - self._known)
        removed = sorted(self._known - snapshot)
        self._known = snapshot
        events = [
            {"event_type": EVENT_BOT_CREATED, "bot_id": client_id,
             "source_timestamp": now}
            for client_id in created
        ] + [
            {"event_type": EVENT_BOT_DISCONNECTED, "bot_id": client_id,
             "source_timestamp": now}
            for client_id in removed
        ]
        self._buffer(events)
        return events

    def _buffer(self, events):
        for event in events:
            if len(self._pending) >= self._max_pending:
                self._dropped += 1
                continue
            self._pending.append(event)

    def take_events(self):
        """Drain the buffered raw events (event-layer side)."""
        events = self._pending
        self._pending = []
        return events

    def dropped_count(self):
        return self._dropped

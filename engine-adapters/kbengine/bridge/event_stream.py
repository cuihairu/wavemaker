"""Event sequencing, cursor, and duplicate dedup (WM-0213,
docs/bridge.md §Event Model).

Turns raw collector records (WM-0209 output: `{event_type, bot_id,
source_timestamp}`) into the full wire envelope, assigns a strictly
increasing per-bridge `sequence`, and keeps a bounded replay window so the
Supervisor can resume from its cursor after a reconnect
(`subscribe_events` command, docs/bridge.md §Command Semantics).

Envelope fields (docs/bridge.md): event_id, backend_id, swell_id, shard_id,
bot_id, sequence, source_timestamp, observed_timestamp, event_type,
payload, schema_version.

Dedup contract:

  * sequence assignment happens in exactly one place (`assign`, the pump's
    sole writer, S1) — strictly increasing, never reused, so a sequence is
    a dedup key on its own;
  * `event_id` is `"<backend_id>:<sequence>"` — deterministic, stable
    across replays; a replayed event carries the identical envelope it was
    first sent with, so Supervisor-side dedup by event_id collapses
    replay/overlap duplicates (the bridge never mutates an assigned
    event);
  * no duplicate production: raw records enter the stream once (the
    collector buffers each observation exactly once).

Cursor semantics: the Supervisor's cursor is the highest sequence it has
durably processed. The replay covers every buffered event with
`sequence > cursor`. The window is bounded — when the cursor falls below
the window's oldest retained sequence the missing range is unrecoverable;
the replay batch then carries `oldest_available` (and `gap`) so the
Supervisor can account for the loss honestly instead of silently
pretending nothing was dropped.

Transport is the caller's concern (the pump): batches are pure message
builders here. An unsolicited batch that cannot be sent (no session,
overflow) is simply dropped — its envelopes stay in the replay window,
so the cursor resume remains the durable delivery path.
"""

import time
from collections import deque

SCHEMA_VERSION = 1
EVENTS_KIND = "events"

# Replay window bound (envelopes). The window only needs to survive a
# Supervisor restart-and-resubscribe; overflow silently evicts the oldest
# envelope and is reported as a gap on replay.
MAX_WINDOW = 4096


class EventStream(object):
    """Sequence assignment + bounded replay window. Scripting-thread only
    (S1: the pump is the sole caller)."""

    def __init__(self, backend_id, swell_id, shard_id_getter,
                 max_window=MAX_WINDOW, clock=None):
        self._backend_id = backend_id
        self._swell_id = swell_id
        self._shard_id_getter = shard_id_getter
        self._clock = time.monotonic if clock is None else clock
        self._max_window = max_window
        self._seq = 0
        self._window = deque(maxlen=max_window)  # (sequence, envelope)
        self._dropped = 0
        self._replay_requested = None  # cursor for the next replay batch

    def assign(self, raw_events):
        """Envelope raw collector records; returns the assigned events.
        Records with no `event_type` are ignored (defensive; collectors
        always set it)."""
        envelopes = []
        shard_id = self._shard_id_getter()
        for raw in raw_events:
            event_type = raw.get("event_type")
            if not event_type:
                continue
            self._seq += 1
            envelope = {
                "event_id": "{0}:{1}".format(self._backend_id, self._seq),
                "sequence": self._seq,
                "backend_id": self._backend_id,
                "swell_id": self._swell_id,
                "shard_id": shard_id,
                "bot_id": raw.get("bot_id"),
                "event_type": event_type,
                "payload": raw.get("payload") or {},
                "source_timestamp": raw.get("source_timestamp"),
                "observed_timestamp": self._clock(),
                "schema_version": SCHEMA_VERSION,
            }
            envelopes.append(envelope)
            if len(self._window) >= self._max_window:
                self._dropped += 1  # oldest falls off; newest still queued
            self._window.append((self._seq, envelope))
        return envelopes

    def build_batch(self, envelopes):
        """Events message for one tick's envelopes (None when empty). The
        cursor is the batch's highest sequence."""
        if not envelopes:
            return None
        highest = max(envelope["sequence"] for envelope in envelopes)
        return {
            "kind": EVENTS_KIND,
            "backend_id": self._backend_id,
            "events": envelopes,
            "cursor": highest,
        }

    def request_replay(self, cursor):
        """Record a `subscribe_events` cursor; the pump sends the replay
        batch on its next tick."""
        self._replay_requested = cursor

    def build_replay_if_requested(self):
        """Replay batch for a requested cursor (None when none requested).

        Replays every buffered event after the cursor; when the cursor is
        below the window's oldest retained sequence the gap is reported
        via `oldest_available` (the unrecoverable range is
        [cursor+1, oldest_available-1]) — never silently skipped.
        """
        cursor = self._replay_requested
        if cursor is None:
            return None
        self._replay_requested = None
        batch = {
            "kind": EVENTS_KIND,
            "backend_id": self._backend_id,
            "events": [
                envelope for sequence, envelope in self._window
                if sequence > cursor
            ],
            "cursor": self._seq,
        }
        if self._window:
            oldest = self._window[0][0]
            if cursor < oldest - 1:
                batch["gap"] = True
                batch["oldest_available"] = oldest
        elif cursor < self._seq:
            batch["gap"] = True
            batch["oldest_available"] = self._seq + 1
        return batch

    def dropped_count(self):
        return self._dropped

    @property
    def last_sequence(self):
        return self._seq

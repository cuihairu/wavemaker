"""Thin, testable port over the KBEngine scripting module.

The bootstrap touches exactly three engine APIs (audit §1/§3/§4):

    KBEngine.callback(delay_seconds, fn) -> callback_id
        Exactly two arguments; single-shot; the engine forwards no extra args
        to `fn`, so callers must use functools.partial to close over state.

    KBEngine.cancelCallback(callback_id)
        Exactly one int argument; silently succeeds on unknown or already
        fired ids (no exception, no feedback that a race happened).

    KBEngine.addBots(totalCount)
        The 1-arg form (1 or 3 args accepted, never 2 — TypeError otherwise).
        Cumulative creation target paced by the engine's own gate (S8);
        returns None with no failure feedback (audit §1/D1).

Bot/entity inspection APIs live in their own collectors (WM-0208+). Wrapping
the module keeps the whole adapter importable and testable without a
KBEngine runtime.
"""


class EnginePort(object):
    """Production port over the live `KBEngine` module."""

    def __init__(self, kbengine_module):
        self._kb = kbengine_module

    @classmethod
    def default(cls):
        # Imported lazily: the module only exists inside the engine process.
        import KBEngine

        return cls(KBEngine)

    def schedule(self, delay_seconds, fn):
        """Register a single-shot callback; returns the engine callback id."""
        return self._kb.callback(delay_seconds, fn)

    def cancel(self, callback_id):
        """Cancel a registered callback id; silent for unknown/fired ids."""
        self._kb.cancelCallback(callback_id)

    def add_bots(self, total_count):
        """Queue `total_count` more bot creations (cumulative, audit §1)."""
        return self._kb.addBots(total_count)


class FakeEngine(object):
    """Scriptable stand-in for `KBEngine` in unit tests.

    Mirrors the real contract precisely: a single-shot 2-arg callback that
    takes a delay and a callable, returns an int id, and never forwards extra
    args to the callable. Tests drive the pump by calling `fire` manually.
    """

    def __init__(self):
        self._next_id = 1
        self.scheduled = {}
        self.cancelled = []
        self.add_bots_calls = []
        self.bots_total = 0

    def callback(self, delay_seconds, fn):
        cid = self._next_id
        self._next_id += 1
        self.scheduled[cid] = (delay_seconds, fn)
        return cid

    def cancelCallback(self, cid):
        self.cancelled.append(cid)
        self.scheduled.pop(cid, None)

    def addBots(self, total_count):
        """Cumulative creation target, mirroring the engine semantics
        (audit §1): records each call and the running total."""
        self.add_bots_calls.append(total_count)
        self.bots_total += total_count
        return None

    def fire(self, cid):
        """Invoke the scheduled callback `cid` exactly once (single-shot)."""
        _, fn = self.scheduled.pop(cid)
        fn()

    def fire_due(self, delay_seconds):
        """Fire every callback whose delay is <= `delay_seconds`; FIFO."""
        due = [
            cid
            for cid, (delay, _fn) in sorted(self.scheduled.items())
            if delay <= delay_seconds
        ]
        for cid in due:
            self.fire(cid)

"""Backend lifecycle: Drain / Shutdown (WM-0211, docs/bridge.md §Command
Semantics).

Semantics and honest boundaries:

  * Drain — "wait for or clean up existing bots according to policy",
    idempotent per operation_id. The bridge marks the backend `draining`
    (one-way) and freezes its own batch schedule (no new bots); the
    session stays live so events keep flowing. The bridge has no
    statically-known API to close individual bot connections (pybots is
    read-only; addBots is the only injection point, audit §2), so the
    waiting/cleanup judgement runs on the Supervisor side against the
    collected bot counts and connection events (WM-0208/0209 outputs).
    Drain implies the StopAdmission gate (no new bot-creating execution
    while draining).
  * Shutdown — "end the Backend instance", idempotent. The bridge ends its
    own protocol session: the ack result is sent on the current tick and
    the session teardown (inline bounded close, pump cancel) runs on the
    NEXT pump tick — one tick of healthy-sender flush time for the ack;
    a stalled socket still counts it as dropped_outbound (§8.3 #5: no
    async teardown, every message is exactly one of sent/dropped/
    encode-error). The bots process itself keeps running: process
    termination belongs to the Supervisor's process supervision
    (crates/wm-supervisor), not to a script-side exit.
  * onFinish interplay (audit §6/D7): these paths never depend on a
    callback succeeding; the teardown is synchronous and pump-driven.
"""

STATE_ACTIVE = "active"
STATE_DRAINING = "draining"
STATE_STOPPED = "stopped"

REASON_ALREADY_DRAINING = "already_draining"
REASON_ALREADY_STOPPED = "already_stopped"


def _fail(code, reason, detail):
    return (code, reason, detail)


class BackendLifecycle(object):
    """One-way lifecycle state machine (scripting-thread only, S1)."""

    def __init__(self):
        self._state = STATE_ACTIVE
        self._drain_operation_id = ""

    @property
    def state(self):
        return self._state

    @property
    def drain_operation_id(self):
        return self._drain_operation_id

    def drain(self, operation_id):
        """Handle a `drain` command. Returns a result tuple."""
        if self._state == STATE_STOPPED:
            return _fail(
                "FAILED_PRECONDITION", REASON_ALREADY_STOPPED,
                "backend already shut down",
            )
        if self._state == STATE_DRAINING:
            if operation_id == self._drain_operation_id:
                return ("OK", "", "")  # idempotent replay of the drain
            return _fail(
                "FAILED_PRECONDITION", REASON_ALREADY_DRAINING,
                "drain {0} in progress".format(self._drain_operation_id),
            )
        self._state = STATE_DRAINING
        self._drain_operation_id = operation_id
        return ("OK", "", "")

    def shutdown(self, operation_id):
        """Handle a `shutdown` command. Returns a result tuple. Idempotent
        without a key (docs/bridge.md §Command Semantics)."""
        if self._state == STATE_STOPPED:
            return ("OK", "", "")
        self._state = STATE_STOPPED
        return ("OK", "", "")

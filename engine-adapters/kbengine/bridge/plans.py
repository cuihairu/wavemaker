"""ApplyPlan batch plan mapping (WM-0207, docs/bridge.md §Command Semantics).

The Supervisor sends `apply_plan` as a plain EngineCommand whose payload is
the frozen ShardPlan for this Backend (proto `ApplyPlanRequest.plan_payload`
is opaque bytes; on the JSON transport it arrives as a JSON object). The
bridge-side v1 payload shape:

    {
      "swell_id": "sw-1",
      "plan_digest": "sha256:...",
      "batches": [
        {"offset_ms": 0,    "bot_count": 10},
        {"offset_ms": 1000, "bot_count": 20}
      ]
    }

`apply` validates and accepts the plan; `start` maps the accepted plan onto
the engine; `due_batches` yields the bot counts whose time bucket has
arrived, one `addBots(totalCount)` call per batch per pump tick.

Semantics honoured here:

  * Idempotency (docs/bridge.md): ApplyPlan replays OK for the same
    plan_digest while accepted; a different digest after acceptance is
    FAILED_PRECONDITION `conflicting_plan` — a frozen shard must never be
    silently replaced (spectrum.md §ShardPlan Freezing). Start is
    idempotent for the same operation_id (replay OK without re-execution).
  * Ordering (docs/bridge.md §Command Semantics): Start before an accepted
    plan is FAILED_PRECONDITION `plan_not_accepted`, carrying the current
    state in the detail (protobuf-conventions §3.2).
  * Batch budget (audit §1/§7 D2): `addBots` has no cap and cumulative
    semantics, so the bridge never fires more than one addBots per batch
    per pump tick and paces batches by their time buckets — arrivals then
    land on the engine's own creation gate (S8: 1 s gate + gameTick
    quantum), not in a tight loop.
  * Time buckets: offsets are quantized to the engine callback quantum
    (SCHEDULING_RESOLUTION_MS, audit §3/D4: exact_arrival=degraded); same
    bucket batches coalesce by summing bot counts.

All execution happens on the scripting thread via the pump (§8.3 #1): the
pump asks `due_batches` every tick; no extra engine callbacks are armed
(§8.3 #3: fewer live handles to cancel).
"""

import time

# Stable reason codes returned alongside the result codes (docs/spec/
# protobuf-conventions.md §3.2: every error carries an explanatory
# reason_code and detail).
OK = ("OK", "", "")
REASON_PLAN_NOT_AN_OBJECT = "plan_not_an_object"
REASON_SWELL_ID_MISSING = "swell_id_missing"
REASON_SWELL_MISMATCH = "swell_mismatch"
REASON_PLAN_DIGEST_MISSING = "plan_digest_missing"
REASON_BATCHES_MISSING = "batches_missing"
REASON_BAD_BATCH = "bad_batch"
REASON_CONFLICTING_PLAN = "conflicting_plan"
REASON_PLAN_NOT_ACCEPTED = "plan_not_accepted"
REASON_ALREADY_RUNNING = "already_running"
REASON_ADMISSION_STOPPED = "admission_stopped"


def _is_int(value):
    # proto integers arrive as JSON numbers; bools are ints in Python but
    # not valid counts.
    return isinstance(value, int) and not isinstance(value, bool)


def _fail(code, reason, detail):
    return (code, reason, detail)


def parse_plan_payload(payload, resolution_ms):
    """Validate the ShardPlan payload; return the normalized plan dict or
    a `(code, reason_code, detail)` rejection tuple.

    Normalization quantizes every batch offset onto the engine callback
    quantum (offset_ms // resolution_ms) and coalesces same-bucket entries
    by summing bot counts, producing `buckets`: a sorted list of
    `(bucket_index, bot_count)` pairs with bucket 0 firing at start.
    """
    if not isinstance(payload, dict):
        return _fail(
            "INVALID_ARGUMENT", REASON_PLAN_NOT_AN_OBJECT,
            "plan payload must be a JSON object",
        )
    swell_id = payload.get("swell_id")
    if not isinstance(swell_id, str) or not swell_id:
        return _fail("INVALID_ARGUMENT", REASON_SWELL_ID_MISSING,
                     "plan payload lacks a non-empty swell_id")
    digest = payload.get("plan_digest")
    if not isinstance(digest, str) or not digest:
        return _fail("INVALID_ARGUMENT", REASON_PLAN_DIGEST_MISSING,
                     "plan payload lacks a non-empty plan_digest")
    batches = payload.get("batches")
    if not isinstance(batches, list) or not batches:
        return _fail("INVALID_ARGUMENT", REASON_BATCHES_MISSING,
                     "plan payload lacks a non-empty batches list")
    buckets = {}
    for index, batch in enumerate(batches):
        if not isinstance(batch, dict):
            return _fail("INVALID_ARGUMENT", REASON_BAD_BATCH,
                         "batches[{0}] is not an object".format(index))
        offset = batch.get("offset_ms")
        count = batch.get("bot_count")
        if not _is_int(offset) or offset < 0:
            return _fail(
                "INVALID_ARGUMENT", REASON_BAD_BATCH,
                "batches[{0}].offset_ms must be a non-negative int".format(
                    index),
            )
        if not _is_int(count) or count < 0:
            return _fail(
                "INVALID_ARGUMENT", REASON_BAD_BATCH,
                "batches[{0}].bot_count must be a non-negative int".format(
                    index),
            )
        bucket = offset // resolution_ms
        buckets[bucket] = buckets.get(bucket, 0) + count
    return {
        "swell_id": swell_id,
        "plan_digest": digest,
        "buckets": sorted(buckets.items()),
    }


class PlanState(object):
    """Accepted-plan bookkeeping for one Backend instance (scripting-thread
    only, S1 — the pump is the sole caller)."""

    def __init__(self, swell_id, resolution_ms, clock=None):
        self._swell_id = swell_id
        self._resolution_ms = resolution_ms
        self._clock = time.monotonic if clock is None else clock
        self._state = "idle"
        self._digest = ""
        self._buckets = []
        self._start_operation_id = ""
        self._started_at = 0.0
        self._fired_through = -1
        self._admission_stopped = False

    @property
    def state(self):
        return self._state

    @property
    def plan_digest(self):
        return self._digest

    @property
    def start_operation_id(self):
        return self._start_operation_id

    def apply(self, payload):
        """Handle an `apply_plan` payload. Returns a
        `(code, reason_code, detail)` result tuple."""
        if self._state == "running":
            # A frozen, executing shard is never replaced mid-run; retry
            # after Drain/Shutdown (spectrum.md §ShardPlan Freezing).
            return _fail(
                "FAILED_PRECONDITION", REASON_CONFLICTING_PLAN,
                "plan {0} is running; apply refused".format(self._digest),
            )
        parsed = parse_plan_payload(payload, self._resolution_ms)
        if isinstance(parsed, tuple):
            return parsed
        if parsed["swell_id"] != self._swell_id:
            return _fail(
                "INVALID_ARGUMENT", REASON_SWELL_MISMATCH,
                "plan targets swell {0!r}, this backend serves {1!r}".format(
                    parsed["swell_id"], self._swell_id),
            )
        digest = parsed["plan_digest"]
        if self._state == "plan_accepted" and digest == self._digest:
            return OK  # idempotent replay of the accepted plan
        if self._state == "plan_accepted":
            return _fail(
                "FAILED_PRECONDITION", REASON_CONFLICTING_PLAN,
                "plan {0} already accepted; refusing {1}".format(
                    self._digest, digest),
            )
        self._digest = digest
        self._buckets = parsed["buckets"]
        self._state = "plan_accepted"
        return OK

    def start(self, operation_id):
        """Handle a `start` command. Returns a result tuple."""
        if self._admission_stopped:
            # StopAdmission is one-way (docs/bridge.md has no reopen): no
            # new bot-creating execution may begin after it.
            return _fail(
                "FAILED_PRECONDITION", REASON_ADMISSION_STOPPED,
                "admission stopped; no new execution",
            )
        if self._state == "idle":
            return _fail(
                "FAILED_PRECONDITION", REASON_PLAN_NOT_ACCEPTED,
                "no plan accepted; apply_plan first",
            )
        if self._state == "running":
            if operation_id == self._start_operation_id:
                return OK  # idempotent replay of the running start
            return _fail(
                "FAILED_PRECONDITION", REASON_ALREADY_RUNNING,
                "start {0} is already running".format(
                    self._start_operation_id),
            )
        self._state = "running"
        self._start_operation_id = operation_id
        self._started_at = self._clock()
        self._fired_through = -1
        return OK

    def stop_admission(self, operation_id):
        """Handle a `stop_admission` command: no new bots from this bridge.

        Stops the bridge's own schedule (remaining batches freeze) and any
        future start. Existing bots are untouched — that is the command's
        semantics (docs/bridge.md: stop admissions, keep connections), and
        the engine's own create-and-login maintenance is outside bridge
        control (audit §8.1 S8 note: the handler recreates kicked bots to
        maintain its target; no statically-known API cancels that — the
        gap is a runtime-verification item for WM-0216).

        Idempotent without a key (docs/bridge.md §Command Semantics):
        every further stop returns OK.
        """
        self._admission_stopped = True
        return OK

    @property
    def admission_stopped(self):
        return self._admission_stopped

    def due_batches(self):
        """Bot counts of every batch whose time bucket has arrived since
        the last call (ascending bucket order). Empty when idle/accepted,
        after StopAdmission (frozen, never resumed), or when the schedule
        is exhausted. One list element = one addBots call (D2 batch
        budget: never more than one per pump tick... per batch)."""
        if self._state != "running" or self._admission_stopped:
            return []
        elapsed_ms = (self._clock() - self._started_at) * 1000.0
        current = int(elapsed_ms // self._resolution_ms)
        if current <= self._fired_through:
            return []
        due = [
            count
            for bucket, count in self._buckets
            if self._fired_through < bucket <= current
        ]
        self._fired_through = current
        return due

"""EngineCommand dispatch layer (WM-0206, docs/bridge.md §Command Queue).

The IPC receiver enqueues raw dicts into the CommandQueue (§8.3 #1); the
bootstrap pump drains them on the scripting thread and hands each one to
`dispatch_command` here. This module is the connective tissue:

    IPC receiver -> CommandQueue -> pump (scripting thread)
                                       -> dispatch_command
                                          -> registered handler
                                       -> EngineCommandResult
                                    -> IPC sender (non-blocking enqueue)

Wire shapes mirror `wavemaker.engine.v1` (proto/engine/v1/engine_bridge.proto
`EngineCommand` / `EngineCommandResult`) and the result envelope mirrors
`wavemaker.common.v1.OperationResult` with stable string codes — the JSON
transport carries enum names, not proto numbers (same convention as the
handshake). `payload` is the JSON stand-in for the proto `bytes` field: any
JSON value the Supervisor sent.

Semantics honoured here (docs/spec/protobuf-conventions.md):

  * Commands are only accepted after the handshake assigned a generation;
    anything arriving earlier fails with FAILED_PRECONDITION (`not_ready`).
  * A command claiming a different generation than the assigned one is
    stale (§2.4: late traffic from an old generation must not take effect)
    and fails with FAILED_PRECONDITION (`stale_generation`).
  * Unknown command names fail with UNSUPPORTED — never a silent no-op.
  * A handler exception is converted into INTERNAL instead of escaping:
    the pump's S3 net still exists, but dispatch reports the failure as a
    result so the Supervisor sees it.

Idempotency (docs/bridge.md §Command Semantics) is per-command-key work
(Start/Drain by operation_id, ApplyPlan by plan_digest) and belongs to the
handlers (WM-0207+); this layer only routes and envelopes.
"""

COMMAND_KIND = "command"
COMMAND_RESULT_KIND = "command_result"

# Stable result codes: the names of wavemaker.common.v1.ErrorCode (JSON
# transport carries names, not proto numbers).
RESULT_OK = "OK"
RESULT_INVALID_ARGUMENT = "INVALID_ARGUMENT"
RESULT_FAILED_PRECONDITION = "FAILED_PRECONDITION"
RESULT_UNSUPPORTED = "UNSUPPORTED"
RESULT_INTERNAL = "INTERNAL"


def parse_command(message):
    """EngineCommand-shaped dict -> `(operation_id, generation, command_name,
    payload)`; None for anything else (S3: message-level surprises never
    raise into the scripting thread).

    Mirrors the proto fields: operation_id/command_name are strings,
    generation an int (bool excluded), payload any JSON value (proto
    `bytes` arrives as a JSON value here). Missing optional fields take
    proto3 defaults; only `kind` must identify the message.
    """
    if not isinstance(message, dict) or message.get("kind") != COMMAND_KIND:
        return None
    operation_id = message.get("operation_id")
    if not isinstance(operation_id, str):
        operation_id = ""
    generation = message.get("generation", 0)
    if not isinstance(generation, int) or isinstance(generation, bool):
        generation = 0
    command_name = message.get("command_name")
    if not isinstance(command_name, str):
        command_name = ""
    payload = message.get("payload")
    return operation_id, generation, command_name, payload


def build_command_result(operation_id, generation, code, reason_code="",
                         detail=""):
    """EngineCommandResult-shaped dict mirroring the proto fields.

    `code` is one of the RESULT_* names of wavemaker.common.v1.ErrorCode;
    every non-OK result must carry a stable `reason_code` and explanatory
    `detail` (docs/spec/protobuf-conventions.md §3.2: a bare numeric code
    is not allowed).
    """
    return {
        "kind": COMMAND_RESULT_KIND,
        "operation_id": operation_id,
        "generation": generation,
        "result": {
            "code": code,
            "reason_code": reason_code,
            "detail": detail,
            "generation": generation,
        },
    }


class CommandDispatcher(object):
    """Routes parsed commands to handlers; produces result envelopes.

    One instance per Bridge, constructed and used only on the scripting
    thread (the pump is the sole consumer) — no locking needed (S1).

    `handlers` maps command_name -> callable(payload) -> None. Handlers run
    inline on the scripting thread and must stay cheap (they block the
    engine tick); anything heavier belongs in the engine-callback work the
    handler itself schedules.
    """

    def __init__(self, send_result, generation_getter):
        # send_result(result_dict) enqueues a result for the Supervisor
        # (non-blocking, never raises — S2); generation_getter() returns the
        # handshake-assigned generation (0 before the handshake).
        self._send_result = send_result
        self._generation_getter = generation_getter
        self._handlers = {}

    def set_handler(self, command_name, handler):
        """Register the handler for one command name (WM-0207+ calls this)."""
        self._handlers[command_name] = handler

    def dispatch(self, command):
        """Dispatch one parsed `(operation_id, generation, name, payload)`
        tuple and enqueue the EngineCommandResult. Returns the result dict
        (also handed to `send_result`)."""
        operation_id, generation, name, payload = command
        assigned = self._generation_getter()
        if assigned == 0:
            result = build_command_result(
                operation_id, assigned, RESULT_FAILED_PRECONDITION,
                "not_ready", "command before handshake completion",
            )
        elif generation != assigned:
            # Stale-generation guard (protobuf-conventions §2.4): late
            # traffic from an old generation never takes effect.
            result = build_command_result(
                operation_id, assigned, RESULT_FAILED_PRECONDITION,
                "stale_generation",
                "command generation {0} != assigned {1}".format(
                    generation, assigned),
            )
        else:
            handler = self._handlers.get(name)
            if handler is None:
                result = build_command_result(
                    operation_id, assigned, RESULT_UNSUPPORTED,
                    "unknown_command", "no handler for {0!r}".format(name),
                )
            else:
                try:
                    handler(payload)
                    result = build_command_result(
                        operation_id, assigned, RESULT_OK)
                except Exception as exc:  # reported, never raised (S3)
                    result = build_command_result(
                        operation_id, assigned, RESULT_INTERNAL,
                        "handler_error", repr(exc),
                    )
        self._send_result(result)
        return result

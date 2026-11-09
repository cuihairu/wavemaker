"""Bridge handshake protocol (WM-0203, docs/bridge.md §Handshake).

First message on the wire after the IPC client connects: the Bridge
announces itself and its honest capability catalog; the Supervisor replies
with an ack that accepts or refuses the session.

Field names mirror `wavemaker.engine.v1` (proto/engine/v1/engine_bridge.proto
`EngineHandshake` / `EngineHandshakeAck`); the ack's `result` mirrors
`wavemaker.common.v1.OperationResult` with stable string codes (the JSON
transport carries names, not proto numbers).

Refusal semantics (docs/bridge.md: "When protocol versions are incompatible,
startup must be refused; silent degradation is not allowed"): a non-OK ack
puts the Bridge into the `refused` state — the IPC client is torn down, no
further protocol traffic flows, and the engine process itself keeps running
(a refusal must never crash bots startup, audit §5).
"""

from .capability import PROTOCOL_VERSION

HANDSHAKE_KIND = "handshake"
HANDSHAKE_ACK_KIND = "handshake_ack"


def build_handshake(report):
    """Bridge -> Supervisor handshake from the bootstrap's capability report.

    `report` is `Bootstrap.capability_report()`; the payload carries the full
    catalog so the Supervisor can validate capabilities, not just versions.
    """
    return {
        "kind": HANDSHAKE_KIND,
        "protocol_version": report["protocol_version"],
        "engine_name": report["engine_name"],
        "engine_version": report["engine_version"],
        "adapter_name": report["adapter_name"],
        "adapter_version": report["adapter_version"],
        "backend_id": report["backend_id"],
        "capabilities": report["capabilities"],
        "scheduling_resolution_ms": report["scheduling_resolution_ms"],
        "arrival_batch_ms": report["arrival_batch_ms"],
    }


def parse_handshake_ack(message):
    """Supervisor -> Bridge ack. Returns `(accepted, generation, reason)`.

    `accepted` is True only for an OK result; `reason` is the Supervisor's
    stable reason code (empty on accept). Anything that is not an ack-shaped
    dict returns `None` — the caller records it and moves on (S3 discipline:
    message-level surprises never raise into the scripting thread).
    """
    if not isinstance(message, dict) or message.get("kind") != HANDSHAKE_ACK_KIND:
        return None
    result = message.get("result")
    if not isinstance(result, dict):
        return (False, 0, "malformed_ack")
    accepted = result.get("code") == "OK"
    generation = message.get("generation", 0)
    if not isinstance(generation, int) or isinstance(generation, bool):
        generation = 0
    reason = "" if accepted else str(
        result.get("reason_code") or result.get("detail") or "refused"
    )
    return (accepted, generation, reason)

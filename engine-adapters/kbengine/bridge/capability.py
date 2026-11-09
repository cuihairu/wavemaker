"""Bridge capability catalog (docs/bridge.md "Capability Model").

The catalog is the honest handshake payload: every value is derived from the
KBEngine source audit (docs/spec/kbengine-api-audit.md §8.1 S1–S8) plus the
no-unverified-claims rule in docs/kbengine.md. Values use the bridge.md
vocabulary: supported | unsupported | unknown | degraded.

Nothing here is guessed. Where the source audit does not establish a fact the
value stays `unknown`; where it establishes a bounded-but-imprecise fact the
value is `degraded` with the measured bound carried alongside.
"""

# Engine Bridge wire protocol version this bootstrap implements.
PROTOCOL_VERSION = 1

# Default engine callback quantum (audit §3): gameUpdateHertz = 10 → 100 ms.
SCHEDULING_RESOLUTION_MS = 100

# addBots pacing (audit §8.1 S8): a fixed 1 s repeat timer opens the gate,
# then up to tickCount bots per second; arrivals land on the next gameTick.
ARRIVAL_BATCH_MS = 1000

CAPABILITIES = {
    "process": {
        "start": "supported",
        "stop": "supported",
        "restart": "supported",
        "resource_limits": "supported",
    },
    "bots": {
        "create_batch": "supported",
        "create_individual": "unknown",
        "destroy_individual": "unknown",
        "pause_individual": "unknown",
        "inspect_individual": "unknown",
    },
    "scheduling": {
        # S8: arrivals are batched (1 s gate + gameTick quantum), so exact
        # arrival is not achievable — but the bound is known, hence degraded
        # rather than unsupported. The two fields carry the actual precision.
        "exact_arrival": "degraded",
        "batch_arrival": "supported",
        "resolution_ms": SCHEDULING_RESOLUTION_MS,
        "arrival_batch_ms": ARRIVAL_BATCH_MS,
        "arrival_quantum_ms": SCHEDULING_RESOLUTION_MS,
    },
    "observability": {
        "process_cpu": "supported",
        "process_memory": "supported",
        "bot_count": "supported",
        "connected_count": "unknown",
        "login_success": "unknown",
        "per_bot_events": "unknown",
        "message_latency": "unsupported",
    },
}

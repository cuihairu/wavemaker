"""Rust<->Python protocol conformance harness (WM-0216, docs/testing.md
"Contract / Conformance: Rust Supervisor <-> Python Bridge").

Runs the REAL bridge Bootstrap (no test doubles on the protocol side)
against a FakeEngine stub, driven by the Rust conformance test
(crates/wm-supervisor/tests/bridge_conformance.rs) over a real Unix socket.
The Rust side plays the Supervisor: it validates the handshake, sends
commands, and asserts on results, events, and heartbeats — exercising the
actual framing ([u32 BE length][UTF-8 JSON]), field names, and command
semantics cross-process, not the in-process test doubles.

Usage:  python3 harness.py <ipc_address>

The harness loops the engine pump (fire_due) until the bridge reaches a
terminal state (`stopped` after Shutdown teardown, or `refused`), then
exits 0; a 60 s safety timeout exits 1 so a wedged run can never hang the
suite. Engine-behaviour simulation stays inside the FakeEngine contract:
bots appear asynchronously after Start (audit §1/§8.1 S8 — addBots is a
cumulative target, creations land on later ticks), which is what turns the
plan into connection events on the wire.
"""

import os
import sys
import time

sys.path.insert(
    0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))

from bridge.bootstrap import Bootstrap, BootstrapConfig  # noqa: E402
from bridge.engine_port import EnginePort, FakeEngine  # noqa: E402

PUMP_DELAY_S = 0.1  # the bridge's own quantum (audit §3/D4)
SAFETY_TIMEOUT_S = 60.0
SIMULATED_BOT_ID = 4101


def main():
    if len(sys.argv) != 2:
        print("usage: harness.py <ipc_address>", file=sys.stderr)
        return 2
    fe = FakeEngine()
    config = BootstrapConfig(
        ipc_address=sys.argv[1],
        token="tok",
        swell_id="sw-conf",
        backend_id="kb-conf",
        backend_generation=12,
    )
    bridge = Bootstrap(EnginePort(fe), config)
    bridge.on_init(0)

    bots_added = False
    deadline = time.monotonic() + SAFETY_TIMEOUT_S
    while time.monotonic() < deadline:
        # One pump tick per loop: fire the armed single-shot callback
        # (the pump always re-arms itself with the same delay).
        fe.fire_due(PUMP_DELAY_S)
        if not bots_added and bridge.plans.state == "running":
            # The engine surfaces created bots on later ticks than the
            # addBots call (S8 creation gate); the next poll diffs them
            # into bot_created events.
            fe.bots.add(SIMULATED_BOT_ID)
            bots_added = True
        if bridge.state in ("stopped", "refused"):
            break
        time.sleep(0.005)
    else:
        print("harness safety timeout expired", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

# KBEngine Bridge (Phase 2)

Python adapter that lets wavemaker manage and observe KBEngine native bots.
The engine-side `bridge` package is loaded by the bots process and driven by
the engine's own Python entry points; the Supervisor side is Rust (Phase 2,
WM-0204+).

## Layout

```text
engine-adapters/kbengine/
├── kbemain.py              # engine entry script: onInit/onFinish -> bridge
├── bridge/
│   ├── __init__.py         # process-wide bootstrap factory
│   ├── bootstrap.py        # WM-0201: state machine + pump + config
│   ├── capability.py       # honest capability catalog (handshake payload)
│   ├── command_queue.py    # thread-safe inbound queue (IPC threads enqueue)
│   ├── ipc_client.py       # WM-0202: local IPC client (UDS/TCP, framing)
│   └── engine_port.py      # thin port over KBEngine.callback/cancelCallback
└── tests/
    ├── __init__.py
    ├── test_bootstrap.py   # WM-0201 unit tests via a FakeEngine port
    └── test_ipc_client.py  # WM-0202 unit tests over real loopback sockets
```

## Loading into KBEngine

Point `<bots><entryScriptFile>` at this module (the default is `kbemain`,
audit §5) or `import kbemain` from your own entry script. The engine calls
`onInit(0)` once before the main loop and `onFinish()` during finalise; both
are delegated to the process-wide bootstrap.

## Configuration injection

The Supervisor sets these environment variables on the bot child process
(kbengine.md lifecycle step 3; WM-0205 may extend the channel):

| Variable | Meaning |
|---|---|
| `WAVEMAKER_BRIDGE_IPC` | local IPC address (empty = standby, no IPC) |
| `WAVEMAKER_BRIDGE_TOKEN` | IPC authentication token |
| `WAVEMAKER_SWELL_ID` | current Swell id |
| `WAVEMAKER_BACKEND_ID` | Backend instance id |

`WAVEMAKER_BRIDGE_IPC` accepts `unix:<path>`, a bare absolute path (the
usual `/tmp/wm.sock`), or `tcp:<host>:<port>` for loopback debugging
(docs/bridge.md). With an injected address, `onInit` starts the local IPC
client (WM-0202) and `onFinish` closes it inline; a connect failure is
recorded on the client (`stats()`) instead of aborting engine startup.
Without an IPC address the bootstrap stays in `standby`: it arms the pump
but performs no I/O.

## Wire format

`[u32 BE length][UTF-8 JSON payload]` — the same length prefix as the Rust
framing layer (crates/wm-runtime/src/framing.rs). JSON rather than protobuf
because the KBEngine embedded interpreter ships no protobuf bindings;
message field names mirror `wavemaker.engine.v1`
(proto/engine/v1/engine_bridge.proto) so the Supervisor side (WM-0204+)
maps messages 1:1. Handshake payload and version refusal is WM-0203.

## Engine constraints honoured (audit §8.3)

1. IPC receive threads only enqueue; the engine-callback pump (100 ms
   default quantum) is the sole consumer on the scripting thread.
2. (WM-0208+) `KBEngine.bots` traversal must snapshot `keys()` and guard each
   entry with `isDestroyed` — no direct iteration of the live map.
3. Callback ids are the engine's own; the pump cancels only its current live
   handle on finish (callbacks are single-shot; a fired handle is never
   cancelled, so no cancel-race feedback is relied on).
4. `exact_arrival` is `degraded` with `arrival_batch_ms=1000` +
   `arrival_quantum_ms=100` (audit §8.1 S8: 1 s batch gate + gameTick).
5. `onFinish` is synchronous (no async teardown armed); the state machine
   never depends on a callback having succeeded — script exceptions are
   printed and swallowed by the engine (S3).

## Tests

```sh
cd engine-adapters/kbengine && python3 -m pytest tests -q
```

Runtime conformance against a real KBEngine (callback latency, addBots
pacing under load) is WM-0216 / RT1, blocked on a deployed engine cluster.
A runnable end-to-end login load test sample on a real cluster is
examples/login_load (WM-0217).

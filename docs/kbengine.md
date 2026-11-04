---
title: KBEngine Adapter
---

# KBEngine Adapter Implementation Design

## Goals

The first Engine Bot Backend targets KBEngine, because:

- It already has native Bots.
- The boundary between the C++ network runtime and Python behavior scripts is clear.
- Existing bot connection and protocol implementations can be reused first.
- It has a Python scripting extension entry point, suitable for implementing the Bridge.
- The C++ client_lib does not need to be linked into the Rust Worker.

## Target Deployment Shape

```text
Host
├── wavemaker-controller
└── wavemaker-supervisor
    ├── KBEngine Bots Process A
    │   ├── KBEngine C++ Runtime
    │   └── Python Bridge
    └── KBEngine Bots Process B
        ├── KBEngine C++ Runtime
        └── Python Bridge
```

The first version does not require Docker. The Supervisor launches the bot binary with a local process launcher. A ContainerProcessLauncher can be added later.

## Python Bridge Modules

Suggested logical files:

```text
engine-adapters/kbengine/
├── bridge/
│   ├── __init__.py
│   ├── bootstrap.py
│   ├── client.py
│   ├── command_queue.py
│   ├── event_collector.py
│   ├── metrics_collector.py
│   ├── bot_registry.py
│   ├── capability.py
│   └── protocol/
├── supervisor/
│   ├── process_spec.rs
│   ├── launcher.rs
│   ├── health.rs
│   └── adapter.rs
├── tests/
│   ├── protocol_conformance/
│   ├── integration/
│   └── failure/
└── README.md
```

This layout is a suggestion; Python code and Rust code are not required to live in the same language build system.

## KBEngine Bridge Lifecycle

1. KBEngine Bots starts.
2. The engine loads the Bridge bootstrap.
3. The Bridge reads the IPC address, token, Swell ID, and Backend ID injected by the Supervisor.
4. The Bridge establishes the IPC connection.
5. It sends Handshake.
6. The Supervisor validates version and capabilities.
7. The Controller sends Initialize.
8. The Supervisor forwards plans and start commands.
9. The Bridge enqueues commands into the engine callback context.
10. The Bridge collects bot states, events, and metrics.
11. When the Swell ends, StopAdmission runs first.
12. Existing bots are drained or awaited according to the DrainPolicy.
13. The Bridge sends final state and metrics.
14. IPC disconnects and the process exits according to policy.

## KBEngine API Verification Checklist

The interfaces below are leads mentioned in ADRs and existing discussions; their semantics must not be inferred from names alone. Source code and runtime behavior must be checked against the version actually in use:

| API / Mechanism | Questions to verify |
|---|---|
| `KBEngine.addBots()` | Parameter meaning, creation pacing, return value, failure callbacks, maximum batch size |
| `KBEngine.bots` | Container type, update timing, stability of bot identity |
| `KBEngine.callback()` | Callback granularity, thread context, maximum lag, exception behavior |
| `KBEngine.cancelCallback()` | Cancellation races and callback handle lifecycle |
| `onInit()` | When the Bridge is initialized, behavior on repeated calls |
| `onFinish()` | Whether it is guaranteed to run on exit, whether asynchronous teardown can be awaited |
| Bot script callbacks | Login success, failure, disconnection, reconnection, and entity state events |
| Process exit | Whether a graceful exit API exists, whether exit codes are reliable |
| Configuration injection | How Swell, Backend, IPC, and authentication parameters are passed |

### No Unverified Capability Claims

For example:

- `destroy_individual` returns `Unknown` until verified.
- `exact_arrival` should be declared `Unsupported` — or `Degraded` with the actual precision — if bots can only be created in callback batches.
- If only the total bot count is available and not the logged-in count, the total must not be presented as online users.
- If per-bot latency cannot be obtained, per-bot latency must not be derived from process-level latency.

## Mode A / Mode B / Mode C

The original ADR access modes should be kept, but unified under the Backend model.

- Mode A: engine-managed bots. Wavemaker controls and observes native bots through the Bridge.
- Mode B: Wavemaker manages the executor but reuses some client capabilities provided by the engine. The exact boundary follows the existing architecture definition; ownership of sockets must not be assumed.
- Mode C: Wavemaker's Native Runtime owns connections and protocol execution itself.

The three modes are not different compile options of the same Actor; they are different combinations of execution backends and access capabilities. The final mapping must follow the Mode A/B/C definitions in the original architecture document; if they are redefined later, a separate ADR is required.

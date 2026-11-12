---
title: Evaluation: Go / C++ / Python Third-Party SDK Completeness
---

# Evaluation: Go / C++ / Python Third-Party SDKs (WM-0611)

> Status: Evaluated — 2026-11-12. This is an evaluation record, not an
> implementation.

## Question

`docs/tech-stack.md:15-20` names six language surfaces (KBEngine Bridge,
BigWorld Bridge, Python SDK, Lua SDK, C++ Adapter, Go SDK) and
`docs/decisions.md:19-22` gives a decision for four of them (Lua, Python,
C++, Go). This evaluation inventories what actually exists in
`engine-adapters/`, produces a completeness matrix, lists the gaps, and
recommends a build priority.

## What `engine-adapters/` actually contains (2026-11-12)

One adapter exists. It is Python. Nothing else is there.

```text
engine-adapters/
└── kbengine/                 # the only adapter in the tree
    ├── kbemain.py            # engine entry script: onInit/onFinish → bridge
    ├── bridge/               # 15 .py files (14 modules + __init__.py)
    │   ├── bootstrap.py      # state machine + pump + config
    │   ├── capability.py     # honest capability catalog (handshake payload)
    │   ├── command_queue.py  # thread-safe inbound queue
    │   ├── commands.py       # command dispatch
    │   ├── connection_events.py
    │   ├── engine_port.py    # thin port over KBEngine.callback/cancelCallback
    │   ├── event_stream.py
    │   ├── handshake.py
    │   ├── ipc_client.py     # local IPC client (UDS/TCP, framing)
    │   ├── lifecycle.py
    │   ├── metrics.py
    │   ├── plans.py
    │   ├── process_metrics.py
    │   └── bot_registry.py
    ├── conformance/harness.py
    ├── tests/                # 10 test modules
    └── examples/             # login_load, rt_measurement (2 e2e drivers)
```

Counts: **32** `.py` source files, **15** `.py` files in `bridge/`
(14 modules + `__init__.py`), **10** test modules, **2** example drivers,
**2** run scripts.

What is **not** there:

| Language | Adapter or SDK | Lines in repo |
|---|---|---|
| Go | none | **0** |
| C++ | none | **0** |
| Rust (general) | none — `Behavior` exists internally but is not exposed | 0 external |
| Lua | none (see WM-0610) | 0 |
| BigWorld | **no adapter, no bridge, no module** | 0 |

The only Go source anywhere in the repository is
`tools/hdr_conformance/go/main.go`, which is a conformance measurement tool —
it is not an SDK and has no dependency on the controller. No `.go`, `.cpp`,
`.cc`, `.hpp`, `.h` or `go.mod` exists under `engine-adapters/` or any
language-SDK path.

BigWorld is a fourth absent surface worth naming separately because
`docs/tech-stack.md:16` marks it "To be verified" rather than absent: the
string "BigWorld" appears in the entire repository only in
`docs/roadmap.md:274`, `docs/tech-stack.md:16`,
`docs/basin.md:114` and `wm-basin/src/overhead.rs:17`. No code, no
conformance harness, no capability catalog. Its status is honest:
undetermined, with the decision deferred to whichever SDK and test
environment materialises.

## The documented language stance (read before comparing)

| Language | `docs/tech-stack.md` | `docs/decisions.md` |
|---|---|---|
| KBEngine Bridge | "Required by the engine scripting environment" | "Python — Required for the KBEngine Bridge" |
| BigWorld Bridge | "Depends on the actual SDK / bot environment — To be verified" | (not listed) |
| Python (general) | "Provided later" | "general SDK later" |
| Lua | "Not implemented in v1" | "Not implemented in v1" |
| C++ | "Separate process or isolated shim" | "Native SDK / separate process / isolated shim" |
| Go | "Provided later, outside the core" | "Not core; SDK possible in the future" |

Two of those rows are load-bearing for this evaluation. First, the Python
row is deliberately split: the **bridge** is required *now* because the engine
scripting environment forces it, while the **general SDK** is "later". Those
are different artifacts and the inventory must not conflate them. Second, Go
is described in two different registers — "provided later" in the tech stack
and "not core; possible in the future" in the decisions — which is closer to
"deprioritized" than "planned". The recommendation below respects that
distinction rather than flattening it.

## What an SDK would actually wrap

The important finding here is that **there are no SDKs yet — only one bridge,
in one language, for one engine.** The completeness question is therefore
mostly a question about what surface is ready to be wrapped.

The existing surface a third-party SDK could plausibly sit on:

| Surface | Location | Shape |
|---|---|---|
| Controller REST | `crates/wm-controller/src/http.rs:47-52` | 5 endpoints: `/healthz`, `/api/v1/status`, `/api/v1/runs` (POST), `/api/v1/runs/{run_id}` (GET), `/api/v1/runs/{run_id}/stop` (POST) |
| Control gRPC | `proto/control/v1/control_service.proto` | `ControlService` — 2 RPCs: `RegisterWorker`, `Heartbeat` |
| Backend gRPC | `proto/backend/v1/backend_service.proto` | `BackendService` — 4 RPCs: `Handshake`, `ApplyPlan`, `Start`, `Stop` |
| Protobuf contracts | `proto/{common,control,backend,engine,metrics}/v1/*.proto` | 5 files |
| `ExecutionBackend` | `docs/backends.md:13-23` | 11 conceptual ops: `handshake`, `initialize`, `apply_plan`, `start`, `update_load`, `stop_admission`, `drain`, `shutdown`, `get_status`, `get_metrics`, `subscribe_events` |
| Native `Behavior` | `crates/wm-runtime/src/behavior.rs:100` | one method, `Box<dyn Behavior>`; no registry, no factory (WM-0610 finding) |

Two of these rows are **not** SDK surfaces, and the distinction matters:

- `ExecutionBackend` is a *conceptual* interface. `docs/backends.md:26` says
  explicitly that "backends are not required to implement them as the same RPC
  methods" and that the protocol must "explicitly describe unsupported
  capabilities". It is a yardstick for auditing an adapter, not an implementable
  contract.
- The `BackendService` gRPC (4 RPCs) covers handshake/plan/start/stop, but
  `StopAdmission` and `Drain` from `docs/backends.md:18-19` have no
  corresponding RPC. `ExecutionBackend` therefore has 11 operations against
  which only 4 are exposed. The gap is structural, not accidental.

A third-party authoring SDK — one that lets an outside user describe a load
pattern and read back results — would wrap the **5 REST endpoints and
plan-as-data**, not the engine bridge. That is the surface the
`docs/tech-stack.md:17-20` rows "Python SDK / Lua SDK / C++ Adapter / Go SDK"
are pointing at. It does not exist in any language.

## Completeness matrix

Rated against the five documented language surfaces, with `docs/backends.md`'s
11-operation `ExecutionBackend` as the yardstick. Cells are: **exists** (code
present and tested), **partial** (present but incomplete against the yardstick),
**absent** (no code), **deferred** (explicitly not in scope).

| Capability | Python | Go | C++ | Lua | Native Rust |
|---|---|---|---|---|---|
| Engine bridge (engine-side scripting) | **exists** — 15 `.py` files in `bridge/`, 10 test modules, 2 e2e examples | absent | absent | absent | n/a — native runtime has no bridge |
| Handshake + capability handshake | **exists** — `handshake.py` + `capability.py`, honest `supported/unsupported/unknown/degraded` catalog | absent | absent | absent | absent |
| `apply_plan` (plan-as-data → engine) | **exists** — `plans.py` | absent | absent | absent | absent |
| `start` / `stop` | **exists** — `commands.py` dispatch | absent | absent | absent | absent |
| `stop_admission` | **absent** — no RPC, no bridge module | absent | absent | absent | absent |
| `drain` | **absent** — no RPC, no bridge module | absent | absent | absent | absent |
| `update_load` | **absent** | absent | absent | absent | absent |
| `get_metrics` (bot-level) | **partial** — process-level exists (`process_metrics.py`); bot-level counts `unknown` in the catalog | absent | absent | absent | absent |
| `subscribe_events` | **partial** — `event_stream.py` present; per-bot events `unknown` | absent | absent | absent | absent |
| IPC / transport | **exists** — `ipc_client.py`, length-prefixed UDS/TCP | absent | absent | absent | absent |
| Conformance harness | **exists** — `conformance/harness.py` | absent | absent | absent | absent |
| General third-party SDK (plan authoring + run read-back) | **absent** | absent | absent | absent | absent |
| Behavior registry (needed by any scripted SDK) | **absent** — WM-0610 finding | absent | absent | absent | absent |

The one column with any content is Python, and even it is partial: 4 of the
11 `ExecutionBackend` operations are absent outright (`stop_admission`,
`drain`, `update_load`, and the bot-level `get_metrics`/`subscribe_events`
splits), because those operations have no RPC on `BackendService` and no
bridge module on the engine side.

### Read the matrix honestly

The empty cells are not a completeness deficit of the Go or C++ columns so much
as they are a statement that **the Go and C++ columns have never started**. The
Python column is the only one where anything has been attempted, and its gaps
are against the conceptual yardstick rather than against the working code.
No language has a general third-party SDK. The premise of "SDK completeness"
is therefore partly false at the point of evaluation: there is one bridge in
one language, and no SDK anywhere.

## Gap list

Ranked by how blocking each one is to a *usable* external SDK, not by how
large it is:

1. **No behavior registry.** `BatchRunner::spawn`
   (`crates/wm-runtime/src/batch.rs:89`) takes a `Box<dyn Behavior>` directly.
   There is no way to *name* a behavior, in any language. This gate is shared
   with WM-0610 and applies to every scripted-SDK row in the matrix.
2. **`BackendService` exposes 4 of 11 `ExecutionBackend` operations.** No RPC
   exists for `StopAdmission` or `Drain` (`docs/backends.md:18-19`). Any
   language's SDK that wishes to respect graceful teardown cannot today.
3. **`update_load` has no surface at all.** It appears in the conceptual
   interface but in neither the gRPC services nor the bridge.
4. **Bot-level observability is `unknown` in the KBEngine catalog.**
   `capability.py` records `connected_count`, `login_success` and
   `per_bot_events` as `unknown` and `message_latency` as `unsupported` —
   honest, but it means a Go or C++ SDK could not promise what the Python
   bridge already disclaims.
5. **Zero Go and C++ code anywhere.** No `go.mod`, no headers, no build. The
   `docs/tech-stack.md:19-20` rows are forward-looking, not in-flight.
6. **BigWorld is undetermined, not absent.** `docs/roadmap.md:274` puts it
   under "v1 Optional" with the qualifier "depending on SDK and test
   environment". That qualifier is still outstanding — there is no source
   review and no measurement against any BigWorld version
   (`docs/basin.md:114` requires exactly that before any capability claim).
7. **Plan-as-data has no typed client.** The 5 REST endpoints accept and
   return JSON; no language binding generates from them. This is the actual
   blocking gap for the *third-party* SDK, and it is orthogonal to which
   language is chosen first.
8. **The adapter's own README documents half of itself.**
   `engine-adapters/kbengine/README.md` (86 lines) has a `Layout` block that
   names only **5 of 14** bridge modules — `bootstrap`, `capability`,
   `command_queue`, `engine_port`, `ipc_client` — omitting `bot_registry`,
   `commands`, `connection_events`, `event_stream`, `handshake`, `lifecycle`,
   `metrics`, `plans` and `process_metrics`. Its `tests/` subtree lists only
   `test_bootstrap` and `test_ipc_client` against **10** test modules that
   actually exist, and the block's per-module comments still cite `WM-0201` /
   `WM-0202` as the newest work in those files — while the module docstrings in
   the tree cite tasks through **`WM-0216`** (`plans.py`,
   `connection_events.py`) and `bootstrap.py` alone spans `WM-0201` →
   `WM-0215`. To be precise: the README's *prose* is current (lines 84-86
   reference `WM-0216` / `WM-0217`), so this is not a stale document — it is a
   **stale diagram**. The authoritative `Layout` block, which is what a
   newcomer reads first, depicts an earlier state of the tree and is the
   specific artifact a completeness review should flag for refresh.

## Recommendation

**Build the Python SDK first, then Go, and defer C++ — because the Python
bridge is the only artifact in the tree that already proves the pattern, and
every gap in it is fixable without new engine access.**

Priority order, with the reason each one is placed where it is:

| # | Target | Rationale |
|---|---|---|
| 1 | **Python general SDK** | The only language with any production code (`bridge/` holds 14 modules plus `__init__.py`, with 10 test modules and 2 e2e drivers). The KBEngine path already demonstrates handshake, capability negotiation, framing, command dispatch and plan application — so the SDK is an *addition* (plan authoring + run read-back over REST) rather than a from-scratch effort. `docs/tech-stack.md:17` already says "provided later"; this is what "later" means. |
| 2 | **Go SDK** | Second, on the strength of `docs/decisions.md:22` ("SDK possible in the future") and the fact that the repo already carries Go tooling (`tools/hdr_conformance/go/`), so the toolchain is not foreign. Wraps the same 5 REST endpoints. Deprioritized by `docs/tech-stack.md:20` ("outside the core") — so it follows Python rather than racing it. |
| 3 | **C++ adapter** | Last, and only when a concrete need exists. `docs/tech-stack.md:19` says "as needed" and `docs/decisions.md:21` says "separate process or isolated shim" — that is a *design policy*, not a commitment. No engine in scope today needs it, and the isolated-shim form means it would be a *new* process boundary rather than an in-process binding. Build it when a named bot framework demands it. |
| — | **Lua** | Deferred — decided separately in WM-0610 (`docs/evaluation/lua-behavior-sdk.md`): mlua chosen, not started, gated on the same behavior registry. |
| — | **BigWorld** | **Undetermined.** Not a priority item. `docs/tech-stack.md:16` marks it "to be verified" and `docs/basin.md:114` requires source review plus measurement on a target version before any capability claim. Until an actual BigWorld SDK and test environment are identified, this row stays open by design. |

### What to unblock first, regardless of language

The blocking work is not language-specific:

1. A **behavior registry** that the runner consumes — needed by any scripted
   SDK, Rust or otherwise (WM-0610 gap #1, shared).
2. **Typed bindings generated from the 5 REST endpoints and the 5 proto
   files** — this is what "SDK" concretely means for a third party, and it
   blocks every column equally.
3. **RPCs for `StopAdmission` and `Drain`** if graceful teardown is to be
   honored from outside the Rust core.

### Where this recommendation changes decisions

- `docs/tech-stack.md:17-20` gains a priority ordering rather than four
  parallel "later/as needed" rows.
- `docs/roadmap.md:274`'s BigWorld row stays explicitly deferred, with the
  condition stated rather than implied.
- `docs/backends.md`'s `ExecutionBackend` remains a *conceptual* yardstick.
  The two missing RPCs are recorded here as a gap rather than silently
  absorbed into "the protocol".

## Caveats

1. **The matrix is a point-in-time inventory, not a forecast.** It reflects
   `engine-adapters/` on 2026-11-12; one new module in `bridge/` changes a
   row.
2. **BigWorld has no verified facts.** Every claim about it here is "not
   yet established", which is itself the finding. `docs/basin.md:114` forbids
   promoting unmeasured assumptions into adapter capabilities, and that rule
   is respected here.
3. **`ExecutionBackend` is not an implementable interface.** It is a
   self-described conceptual contract (`docs/backends.md:26`). Completeness
   measured against it is a *gap measurement*, not a compliance test.
4. **"Go" in the repo is not an SDK.** `tools/hdr_conformance/go/main.go` is a
   benchmarking tool. Counting it as Go SDK progress would be wrong; this
   evaluation does not.
5. **No language has a third-party SDK.** The single clearest finding. The
   matrix's empty cells are accurate and should not be read as an accusation
   against the missing languages — they are a description of what has not
   started.

## Read next

- `docs/backends.md:13-23` — the 11-operation `ExecutionBackend` used as the
  yardstick here
- `engine-adapters/kbengine/README.md` — layout, config injection, wire
  format, and the three engine constraints honoured
- `engine-adapters/kbengine/bridge/capability.py` — the honest
  `supported / unsupported / unknown / degraded` catalog this matrix leans on
- `crates/wm-controller/src/http.rs:47-52` — the 5 REST endpoints an SDK
  would wrap
- `docs/evaluation/lua-behavior-sdk.md` — the companion evaluation for the
  Lua row, including the shared behavior-registry gate

## Conclusion

**One bridge, one language, no SDK.** `engine-adapters/` contains exactly one
adapter — the Python KBEngine bridge — and nothing in Go, C++, Lua, or for
BigWorld. The honest completeness reading is that Python is *partial*
against `ExecutionBackend` (4 of 11 operations absent, mostly because no RPC
exists), while Go, C++ and Lua are *absent* outright and BigWorld is
*undetermined* by design.

Build the Python SDK first, then Go, and defer C++ until a named bot
framework demands it. Unblock the shared prerequisites before choosing a
language: a behavior registry, typed bindings from the existing REST and
proto contracts, and RPCs for `StopAdmission` and `Drain`. BigWorld stays
open until someone reviews its source and measures it — and that is the
correct state, not a gap to be closed on a schedule.

---
title: Evaluation: Lua Behavior SDK
---

# Evaluation: Lua Behavior SDK (WM-0610)

> Status: Evaluated — 2026-11-12. This is an evaluation record, not an
> implementation.

## Question

Three places in `docs/` already say Lua is not in v1, in three different
register. This evaluation decides the two questions they leave open:

1. **If** Lua scripting ever enters, which library is the choice?
2. How well does it fit the behavior model that already shipped in
   `wm-runtime` — and what must exist first before anyone writes a line?

## Existing decision (read first)

| Source | Line | What it says |
|---|---|---|
| `docs/decisions.md` | 19 | `Lua` — "Not implemented in v1" |
| `docs/decisions.md` | 18 | "Behavior in v1 — Internal Rust Behavior" |
| `docs/tech-stack.md` | 18 | `Lua SDK` — "Not implemented in v1" |
| `docs/roadmap.md` | 280 | "Lua sandbox" is listed under **v1 Will Not Include** |
| `docs/roadmap.md` | 285 | "Unlimited embedding of arbitrary language runtimes" is also listed under **v1 Will Not Include** |
| `docs/native-runtime.md` | 97 | "Lua is not in v1. If introduced later, it must come with an instruction budget, sandboxing, deterministic RNG, time injection, and isolation tests." |

The recommendation below does **not** override any of those lines. It answers
the two questions in case the constraint is ever relaxed.

## Ecosystem: rlua vs mlua (verified 2026-11-12)

| Dimension | rlua | mlua |
|---|---|---|
| Latest version | `0.20.1` | `0.12.2` |
| Last release | **2024-07-20** | 2026-10-03 |
| Repository state | **`archived: true`** | active (pushed 2026-11-11) |
| Self-description | "rlua is now deprecated in favour of mlua" — "a thin transitional wrapper around mlua" | "High level bindings to Lua 5.5/5.4/5.3/5.2/5.1 (including LuaJIT) and Luau with async/await features" |
| License | MIT | MIT (`license = "MIT"` in `Cargo.toml`) |
| Downloads (crates.io) | ~0.8M | ~8.8M |
| Stars / open issues | 1756 / 5 | 2943 / 63 |
| Lua VMs | 5.1, 5.3, 5.4, LuaJIT | 5.1–5.5, LuaJIT (`luajit52`), plus **Luau** (optional) |
| Rust edition | — | 2024 |
| `rust-version` | — | **1.88** |

The choice is not close: **rlua is archived, and its own README tells
readers to use mlua instead.** There is no credible case for starting a new
binding on it.

One asymmetry that changes what "add a dependency" means in practice. The
`[features]` table in mlua's `Cargo.toml` defines `lua51`…`lua55`, `luajit`,
`luajit52`, `luau` and `luau-jit`, but **there is no `default = [...]` key**
(the `features` list at line 19 belongs to
`[package.metadata.docs.rs]`, not to `[package]`). So:

```toml
# this does not embed any Lua VM at all — it will fail to build with use sites
mlua = "0.12"

# and this is the actual form a consumer must write
mlua = { version = "0.12", features = ["lua54", "send"] }
```

Every candidate in this evaluation must therefore pin a VM feature.

## Sandbox primitives (the stated requirement, checked against the API)

`docs/native-runtime.md:97` lists five mandatory ingredients. Verified present
in `mlua` 0.12:

| Requirement | API |
|---|---|
| Instruction budget | `Lua::set_hook(HookTriggers, Fn(&Lua, &Debug) -> Result<VmState>)` with `HookTriggers::every_nth_instruction: Option<u32>` |
| Sandboxing | Same hook, plus GC controls (`gc_stop` / `gc_restart` / `gc_collect` / `gc_step` / `gc_set_mode`) |
| Memory bound | `Lua::set_memory_limit(usize) -> Result<usize>` and `used_memory()` |
| Time injection | Not a library feature — `BehaviorContext::now_ns()` already supplies the clock; the library's job is to keep the script from reading wall time instead |
| Deterministic RNG | Not a library feature — `BehaviorContext::rng()` already supplies the stream; the library's job is to keep the script from using the VM's own PRNG |

Two of the five rows above are **library** capabilities and three are
**host** responsibilities. That split is the useful finding in this section:
the binding supplies the budget and the memory cap, but determinism is not
something a Lua binding gives you — it has to be enforced by the adapter, and
by construction it is cheap to do because of how the existing runtime is
shaped (below).

Two qualifications to carry forward, neither of which the API docs advertise:

- `HookTriggers` documents that "setting this option to a low value can incur a
  very high overhead", and the budget counts **VM instructions, not logical
  steps**. A budget constant therefore needs calibration, and the number that
  makes one swell finish must be justified rather than guessed.
- `set_memory_limit` support is VM-specific — the changelog records it as
  "Support setting memory limit for Lua 5.1/JIT/Luau". The exact availability
  under `lua54` is **unverified** by this evaluation and must be established
  at implementation time rather than assumed.

## Fit with the existing behavior model

### The seam is already the right shape

`wm-runtime/src/behavior.rs` defines the whole surface a script must be able
to drive:

```rust
pub trait Behavior {
    fn on_event(&mut self, ctx: &mut BehaviorContext, event: &BehaviorEvent) -> ActorDecision;
}

pub enum BehaviorEvent { Started, Message(u32, Vec<u8>), Timer(u64) }
pub enum ActorDecision { Continue, Wait, Close }
pub enum BehaviorEffect { Send(u32, Vec<u8>), ArmTimer { id: u64, at_ns: u64 }, Log(String) }
```

A `LuaBehavior { lua: Lua, chunk: Chunk }` would implement exactly one method,
invoke one Lua callback, and decode its return value into `ActorDecision`.
The trait is object-safe (`Box<dyn Behavior>`) precisely so the runner holds
it polymorphically — a script-backed behavior is indistinguishable from a
Rust one to the rest of the runtime.

### The collected-effect model is what makes this safe

`BehaviorContext` **collects** effects and never executes them
(`behavior.rs:8-11`): the behavior calls `ctx.send(...)`, which only
`push`es onto a `Vec<BehaviorEffect>`; the batch runner drains them with
`take_effects()` and routes them to transport and scheduler.

That is the single most valuable property of the current design for this
question. The sandbox hands back *intent*, not *action*. A script that calls
`wm.send(1, "x")` cannot open a socket, touch a thread handle, or allocate
outside its own VM — because the transport call happens on the Rust side,
after the effect has been drained, by code the sandbox does not control.
Sandboxing is therefore achieved largely by the runtime's existing structure
rather than bolted on afterwards.

The mapping is one-to-one and complete:

| Lua-side function | Existing effect |
|---|---|
| `wm.send(id, payload)` | `BehaviorEffect::Send(u32, Vec<u8>)` |
| `wm.timer(id, at_ns)` | `BehaviorEffect::ArmTimer { id, at_ns }` |
| `wm.log(msg)` | `BehaviorEffect::Log(String)` |
| `wm.now()` | `ctx.now_ns()` (already injected) |
| `wm.random()` | `ctx.rng().next_f64()` — plus `next_range_f64` / `next_exponential` from `rng.rs` |
| return value | `ActorDecision::{Continue, Wait, Close}` |

### Determinism is preserved by not exposing the VM's PRNG

`BehaviorContext::rng()` returns the runner's `&mut SeededRng`, and
`tick(now_ns, rng)` passes the same stream into every behavior. Replaying a
seed therefore replays the same draw order — **provided the script never
draws from a second, VM-internal source**. So the adapter must not merely
*add* `wm.random()`, it must **remove** the VM's own:

- shadow `math.random` / `math.randomseed` so a script cannot reach the Lua
  VM's PRNG, which is not driven by our seed;
- disable the `os` library, so `os.time`, `os.clock` and `os.date` cannot
  read the wall clock and defeat `wm.now()`.

Neither is hard, but both are **mandatory**, and neither is optional
hardenings: either one silently breaks the reproducibility guarantee
`docs/native-runtime.md` sells. This evaluation deliberately did not soften
that point.

Tellingly, rlua's own `[features]` table contains a commented-out option
marked `# Remove Lua's os lib` (`#lua-no-oslib`, alongside an equally
commented-out `#lua-compat-mathlib`) — i.e. the previous maintainer
identified precisely this hardening need, left it unbuilt, and then archived
the crate. The need is real and well known; mlua leaves it to the host to
implement by hand.

### The one real gap: there is no entry point

`BatchRunner::spawn(now_ns, planned_at_ns, behavior: Box<dyn Behavior>)`
(`batch.rs:89`) takes the behavior it will run. Nothing anywhere in the
runtime — or in `wm-worker` / `wm-controller`, neither of which constructs a
runner yet — can **name** a behavior. There is no behavior registry and no
`from_name` / `by_name` mapping; the only semantic registry in the crate is
`MessageSchema` (`codec.rs:17`), which is about message ids.

So a Lua SDK cannot be added behind an existing seam. Someone must first
decide how a swell names the behavior it wants to run — which is exactly the
plan-as-data question the controller owns. **This prerequisite is independent
of whether the behavior ends up being Rust or Lua**, and it is the item that
gates both.

### Two cheaper engineering gates

- **`rust-version` is 1.88 against a workspace `rust-version = "1.85"`**
  (`Cargo.toml:20`). Adopting mlua 0.12 means either raising the workspace
  MSRV by three minor versions, or pinning an older mlua that still builds on
  1.85. That is a real decision, not a footnote — the exact last 1.85-compatible
  release is not established by this evaluation.
- **`Lua` is not `Send` by default.** The runtime is currently fully
  synchronous (`&mut self`, no `async fn` on the runner) and is not yet
  hosted anywhere, so this costs nothing today. But it is meant to live in a
  worker, and the fix — the opt-in `send` feature — is one feature flag, so it
  is worth paying now rather than discovering it under load. Note `send` is
  **not** a default feature; it has to be named.

## Recommendation

**Pick `mlua` (`0.12.2`) as the sole candidate and reject `rlua` outright —
but do not start it now.** Ship nothing; keep the internal Rust `Behavior`
trait as the v1 path. Revisit only when an external authoring need is
concrete, and when that time comes build the SDK as a **thin adapter that
implements the existing `Behavior` trait** over `BehaviorContext`'s
collected-effect model — never as a parallel scripting execution layer.

The second half is the part that changes work: the gating item is not Lua.
It is a behavior registry, which does not exist and which the runtime is not
wired to use yet.

### Why this shape

1. **rlua is not a candidate, only a historical footnote.** It is archived,
   its last release predates this evaluation by over two years, and its own
   README redirects to mlua. Choosing it would mean maintaining against a
   deprecated crate on purpose.
2. **The runtime is already built for this.** `ActorDecision` is a three-value
   enum, `BehaviorEvent` is a three-variant enum, and effects are collected
   rather than executed — so the sandbox cannot reach the transport directly.
   The adapter is small by construction.
3. **Determinism comes for free from the existing design, if the adapter is
   careful.** `SeededRng` is already the single random source and `now_ns`
   is already injected. The work is *removal* (`math.random`, `os`), not
   *construction* of a new deterministic scheduler.
4. **It keeps the documented scope intact.** `docs/roadmap.md:280` and
   `:285` exclude both the Lua sandbox and "unlimited embedding of arbitrary
   language runtimes" from v1; this recommendation adds neither.

### What must be true before anyone starts

- A behavior registry exists and the runner consumes it (the real gate).
- A VM feature is pinned — `lua54` is the conservative pick; `luajit` and
  `luau` are available and are a genuine choice to argue about, not a detail.
  Luau is the ecosystem choice for game scripting and the plausible long-term
  fit for bot scripts, but it is a different VM with different semantics, and
  `luau` support in the binding is itself optional.
- The workspace MSRV question is answered (1.85 today, 1.88 for mlua 0.12).
- The `send` feature is enabled deliberately, ahead of the runtime moving into
  a worker.
- `wm.random()` / `wm.now()` are provided **and** `math.random` / `os` are
  removed, with a test that asserts the shadowing rather than merely relying
  on it.
- An instruction budget is calibrated against a real swell, and the memory
  limit's availability on the chosen VM is proven, not assumed.

## Read next

- `crates/wm-runtime/src/behavior.rs` — the trait, the events, the
  `ActorDecision`, and the collected-effect model this evaluation leans on
- `crates/wm-runtime/src/rng.rs` — `next_f64` / `next_range_f64` /
  `next_exponential`, the primitives `wm.random()` would wrap
- `crates/wm-runtime/src/batch.rs:89` — `BatchRunner::spawn`, showing that the
  runner takes a behavior directly and that no registry exists
- `docs/native-runtime.md:97` — the five mandatory requirements for any later
  introduction
- mlua `Cargo.toml` — feature table with no `default` key, and
  `rust-version = "1.88"`

## Conclusion

**mlua, not rlua — and not yet.** rlua is archived and self-deprecated, so
the selection is settled; mlua 0.12.2 is the one binding to use, under an
explicit VM feature and with `send`. But the fit finding that matters is
structural: the `Behavior` trait and its collected-effect `BehaviorContext`
make a sandboxed adapter cheap and safe by construction, while the *blocking*
gap is that no behavior registry exists at all. Add the registry first, keep
the internal Rust behavior as v1, and treat a Lua SDK as a thin adapter over
the seam that is already there.

---
title: 评估：Lua Behavior SDK
---

# 评估：Lua 行为 SDK（WM-0610）

> 状态：已评估 —— 2026-11-12。本页是评估记录，不是实现。

## 问题

`docs/` 中已有三处说明 Lua 不在 v1 里，且用了三种不同的表述。本次评估裁决它们留下的两个问题：

1. **如果** Lua 脚本将来要进入，选哪个库？
2. 它与 `wm-runtime` 已经交付的行为模型契合到什么程度 —— 以及在任何人写下第一行代码之前，必须先存在什么？

## 既有决策（先读这里）

| 来源 | 行号 | 内容 |
|---|---|---|
| `docs/decisions.md` | 19 | `Lua` —— "v1 不实现" |
| `docs/decisions.md` | 18 | "v1 的行为 —— 内部 Rust Behavior" |
| `docs/tech-stack.md` | 18 | `Lua SDK` —— "v1 暂不实现" |
| `docs/roadmap.md` | 280 | "Lua 沙箱" 列在 **v1 将不包含** 之下 |
| `docs/roadmap.md` | 285 | "无限制嵌入任意语言运行时" 同样列在 **v1 将不包含** 之下 |
| `docs/native-runtime.md` | 97 | "Lua 不在 v1 中。如果日后引入，必须附带指令预算、沙箱、确定性 RNG、时间注入和隔离测试。" |

下面的推荐**不**推翻上述任何一行。它只是回答那两个问题，以备该约束将来被放宽。

## 生态：rlua vs mlua（2026-11-12 已核实）

| 维度 | rlua | mlua |
|---|---|---|
| 最新版本 | `0.20.1` | `0.12.2` |
| 最近发布 | **2024-07-20** | 2026-10-03 |
| 仓库状态 | **`archived: true`** | 活跃（2026-11-11 有推送） |
| 自述 | "rlua 现已弃用，建议改用 mlua" —— "围绕 mlua 的一个薄过渡封装" | "对 Lua 5.5/5.4/5.3/5.2/5.1（含 LuaJIT）和 Luau 的高层绑定，带 async/await 特性" |
| 许可证 | MIT | MIT（`Cargo.toml` 中的 `license = "MIT"`） |
| 下载量（crates.io） | ~0.8M | ~8.8M |
| Star 数 / 未关闭 issue | 1756 / 5 | 2943 / 63 |
| Lua VM | 5.1、5.3、5.4、LuaJIT | 5.1–5.5、LuaJIT（`luajit52`），另有 **Luau**（可选） |
| Rust edition | — | 2024 |
| `rust-version` | — | **1.88** |

这个选择并不接近：**rlua 已归档，而且它自己的 README 就叫读者去用 mlua。** 在它之上启动一个新绑定没有任何站得住脚的理由。

有一个不对称之处，它改变了"添加一个依赖"在实践中的含义。mlua 的 `Cargo.toml` 中 `[features]` 表定义了 `lua51`…`lua55`、`luajit`、`luajit52`、`luau` 和 `luau-jit`，但**没有 `default = [...]` 键**（第 19 行的 `features` 列表属于 `[package.metadata.docs.rs]`，不属于 `[package]`）。所以：

```toml
# this does not embed any Lua VM at all — it will fail to build with use sites
mlua = "0.12"

# and this is the actual form a consumer must write
mlua = { version = "0.12", features = ["lua54", "send"] }
```

因此本次评估中的每个候选都必须钉死一个 VM feature。

## 沙箱原语（明示的需求，已对照 API 核验）

`docs/native-runtime.md:97` 列出五项必需要素。已在 `mlua` 0.12 中核实存在：

| 需求 | API |
|---|---|
| 指令预算 | `Lua::set_hook(HookTriggers, Fn(&Lua, &Debug) -> Result<VmState>)`，配合 `HookTriggers::every_nth_instruction: Option<u32>` |
| 沙箱化 | 同一 hook，外加 GC 控制（`gc_stop` / `gc_restart` / `gc_collect` / `gc_step` / `gc_set_mode`） |
| 内存上限 | `Lua::set_memory_limit(usize) -> Result<usize>` 与 `used_memory()` |
| 时间注入 | 不是库特性 —— `BehaviorContext::now_ns()` 已经提供时钟；库的职责反而是阻止脚本读取墙钟时间 |
| 确定性 RNG | 不是库特性 —— `BehaviorContext::rng()` 已经提供随机流；库的职责反而是阻止脚本使用 VM 自带的 PRNG |

上表五行中有两行是**库**能力，三行是**宿主**职责。这个划分就是本节有用的结论：绑定提供预算和内存上限，但确定性不是 Lua 绑定能送给你的东西 —— 它必须由适配器来强制，而且从构造上看这样做很便宜，因为现有 runtime 的形状正好如此（见下）。

有两点限制条件要带下去，API 文档对二者都只字未提：

- `HookTriggers` 的文档写明 "把该选项设成较低的值可能带来非常高的开销"，而且预算计数的是 **VM 指令，不是逻辑步**。因此预算常数需要校准，让一次 Swell 跑完的那个数字必须有依据，而不是猜出来的。
- `set_memory_limit` 的支持是按 VM 而定的 —— changelog 把它记为 "为 Lua 5.1/JIT/Luau 支持设置内存上限"。它在 `lua54` 下的确切可用性**未经本评估核实**，必须在实现时确认，不能假设。

## 与既有行为模型的契合度

### 接缝本身已经是正确的形状

`wm-runtime/src/behavior.rs` 定义了一个脚本必须能驱动的全部表面：

```rust
pub trait Behavior {
    fn on_event(&mut self, ctx: &mut BehaviorContext, event: &BehaviorEvent) -> ActorDecision;
}

pub enum BehaviorEvent { Started, Message(u32, Vec<u8>), Timer(u64) }
pub enum ActorDecision { Continue, Wait, Close }
pub enum BehaviorEffect { Send(u32, Vec<u8>), ArmTimer { id: u64, at_ns: u64 }, Log(String) }
```

一个 `LuaBehavior { lua: Lua, chunk: Chunk }` 只需要实现恰好一个方法，调用一个 Lua 回调，并把返回值解码成 `ActorDecision`。这个 trait 是对象安全的（`Box<dyn Behavior>`），正是为了让 runner 以多态方式持有它 —— 对 runtime 的其余部分而言，脚本支撑的行为与 Rust 行为无法区分。

### 收集式 effect 模型才是安全性的来源

`BehaviorContext` **收集** effect 且从不执行它们（`behavior.rs:8-11`）：行为调用 `ctx.send(...)`，它只是 `push` 进一个 `Vec<BehaviorEffect>`；batch runner 用 `take_effects()` 把它们排空，并路由给 transport 和调度器。

对这个问题而言，这是当前设计最有价值的一个性质。沙箱交还的是*意图*，不是*动作*。一个调用 `wm.send(1, "x")` 的脚本无法打开 socket、无法碰线程句柄、也无法在自己的 VM 之外分配内存 —— 因为 transport 调用发生在 Rust 一侧，在 effect 被排空之后，由沙箱控制不了的代码执行。因此沙箱化主要靠 runtime 的既有结构达成，而不是事后加装。

映射是一对一且完备的：

| Lua 侧函数 | 既有 effect |
|---|---|
| `wm.send(id, payload)` | `BehaviorEffect::Send(u32, Vec<u8>)` |
| `wm.timer(id, at_ns)` | `BehaviorEffect::ArmTimer { id, at_ns }` |
| `wm.log(msg)` | `BehaviorEffect::Log(String)` |
| `wm.now()` | `ctx.now_ns()`（已注入） |
| `wm.random()` | `ctx.rng().next_f64()` —— 外加 `rng.rs` 中的 `next_range_f64` / `next_exponential` |
| 返回值 | `ActorDecision::{Continue, Wait, Close}` |

### 不暴露 VM 的 PRNG 就保住了确定性

`BehaviorContext::rng()` 返回 runner 的 `&mut SeededRng`，而 `tick(now_ns, rng)` 把同一条流传给每个行为。因此重放一个种子就重放同一抽值顺序 —— **前提是脚本从不从第二个来源，即 VM 内部的来源抽值**。所以适配器不能只是*加上* `wm.random()`，它必须**移除** VM 自带的：

- 遮蔽 `math.random` / `math.randomseed`，使脚本无法触及 Lua VM 的 PRNG（它不由我们的种子驱动）；
- 禁用 `os` 库，使 `os.time`、`os.clock` 和 `os.date` 无法读取墙钟时间、无法架空 `wm.now()`。

两者都不难，但都是**强制**的，也都不是可选加固：任何一项缺失都会无声地破坏 `docs/native-runtime.md` 所宣称的可复现性保证。本评估刻意没有软化这一点。

很能说明问题的是，rlua 自己的 `[features]` 表里有一条被注释掉的选项，标注 `# Remove Lua's os lib`（`#lua-no-oslib`，旁边还有一条同样被注释掉的 `#lua-compat-mathlib`）—— 也就是说，前任维护者准确识别了这个加固需求，没有实现它，然后归档了这个 crate。这个需求真实且众所周知；mlua 把它留给宿主手工实现。

### 唯一真实的缺口：没有入口

`BatchRunner::spawn(now_ns, planned_at_ns, behavior: Box<dyn Behavior>)`（`batch.rs:89`）直接拿到它要运行的行为。runtime 里没有任何东西 —— `wm-worker` / `wm-controller` 里也没有，二者目前都还没构造 runner —— 能**叫出**一个行为的名字。没有行为注册表，也没有 `from_name` / `by_name` 映射；这个 crate 里唯一的语义注册表是 `MessageSchema`（`codec.rs:17`），而它管的是消息 id。

所以 Lua SDK 无法挂在一个既有接缝后面加进去。必须先有人决定一次 Swell 如何叫出它想运行的行为 —— 这正是 controller 掌管的 plan-as-data 问题。**这个前提与行为最终是 Rust 还是 Lua 无关**，而且正是它把两者都卡住。

### 两个成本更低的工程门槛

- **mlua 的 `rust-version` 是 1.88，对照 workspace 的 `rust-version = "1.85"`**（`Cargo.toml:20`）。采用 mlua 0.12 意味着要么把 workspace MSRV 提高三个次版本，要么钉住一个仍能在 1.85 上构建的更旧 mlua。这是一个真实的决策，不是脚注 —— 确切的最后一个兼容 1.85 的发布版本未经本评估确认。
- **`Lua` 默认不是 `Send`。** runtime 目前完全同步（`&mut self`，runner 上没有 `async fn`），也还没被托管在任何地方，所以今天这不花任何代价。但它注定要活在一个 Worker 里，而修复 —— 即 opt-in 的 `send` feature —— 只是一个 feature flag，所以现在就付这个成本，好过在负载之下才发现。注意 `send` **不是**默认 feature；必须显式写出。

## 推荐

**选定 `mlua`（`0.12.2`）为唯一候选，直接排除 `rlua` —— 但现在不动工。** 不交付任何东西；继续把内部 Rust `Behavior` trait 作为 v1 路径。只有当外部编写需求变得具体时才重新审视，而到那时要把 SDK 构建为在 `BehaviorContext` 收集式 effect 模型之上、**实现既有 `Behavior` trait 的薄适配器** —— 绝不建成一个并行的脚本执行层。

后半句才是改变工作量判断的部分：卡住的项不是 Lua。它是一个行为注册表，它不存在，runtime 也还没接好线去用它。

### 为什么是这个形状

1. **rlua 不是候选，只是一条历史脚注。** 它已归档，最近一次发布比本评估早两年多，它自己的 README 都重定向到 mlua。选它等于故意对着一个已弃用的 crate 做维护。
2. **runtime 已经为此建好了。** `ActorDecision` 是一个三值 enum，`BehaviorEvent` 是一个三变体 enum，effect 是被收集而非被执行的 —— 所以沙箱无法直接触及 transport。适配器从构造上就小。
3. **只要适配器小心，确定性从既有设计里免费得到。** `SeededRng` 已经是唯一随机源，`now_ns` 已经注入。要做的工作是*移除*（`math.random`、`os`），不是*构造*一个新的确定性调度器。
4. **它保持成文范围不变。** `docs/roadmap.md:280` 和 `:285` 已把 Lua 沙箱和"无限制嵌入任意语言运行时"都排除在 v1 之外；本推荐两者都不添加。

### 在任何人动工之前必须成立的条件

- 存在一个行为注册表，且 runner 消费它（真正的门槛）。
- 钉死一个 VM feature —— `lua54` 是保守之选；`luajit` 和 `luau` 可用，而且是值得认真争论的真实选择，不是细节。Luau 是游戏脚本方面的生态选择，也是 bot 脚本看似合理的长期归宿，但它是另一个 VM、另一套语义，而且绑定中的 `luau` 支持本身是可选的。
- workspace MSRV 问题有答案（现在 1.85，mlua 0.12 要 1.88）。
- `send` feature 被有意启用，赶在 runtime 进入 Worker 之前。
- 提供 `wm.random()` / `wm.now()`，**并且**移除 `math.random` / `os`，还要有一个断言遮蔽生效的测试，而不是仅仅依赖它。
- 指令预算对照一次真实 Swell 完成校准，且内存上限在所选 VM 上的可用性被证明，而不是被假设。

## 延伸阅读

- `crates/wm-runtime/src/behavior.rs` —— trait、事件、`ActorDecision`，以及本评估所依赖的收集式 effect 模型
- `crates/wm-runtime/src/rng.rs` —— `next_f64` / `next_range_f64` / `next_exponential`，即 `wm.random()` 将要包装的原语
- `crates/wm-runtime/src/batch.rs:89` —— `BatchRunner::spawn`，说明 runner 直接拿行为，且不存在任何注册表
- `docs/native-runtime.md:97` —— 任何日后引入都必须满足的五项强制需求
- mlua 的 `Cargo.toml` —— 没有 `default` 键的 feature 表，以及 `rust-version = "1.88"`

## 结论

**用 mlua，不用 rlua —— 而且现在还不动。** rlua 已归档并自我弃用，所以选型已定；mlua 0.12.2 是唯一要用的绑定，须在显式 VM feature 之下并加上 `send`。但真正重要的契合结论是结构性的：`Behavior` trait 及其收集式 effect 的 `BehaviorContext` 使沙箱化适配器从构造上就便宜且安全，而*阻塞性的*缺口是行为注册表根本不存在。先加注册表，继续把内部 Rust 行为作为 v1，把 Lua SDK 当作架在既有接缝上的薄适配器。
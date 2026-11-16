---
title: 评估：Go / C++ / Python 第三方 SDK 完备度
---

# 评估：Go / C++ / Python 第三方 SDK（WM-0611）

> 状态：已评估 —— 2026-11-12。本页是评估记录，不是实现。

## 问题

`docs/tech-stack.md:15-20` 点名了六个语言面（KBEngine Bridge、BigWorld Bridge、Python SDK、Lua SDK、C++ Adapter、Go SDK），`docs/decisions.md:19-22` 对其中四个（Lua、Python、C++、Go）给出了决策。本次评估盘点 `engine-adapters/` 里实际存在的东西，产出一个完备性矩阵，列出缺口，并给出构建优先级建议。

## `engine-adapters/` 里实际有什么（2026-11-12）

只有一个 adapter 存在。它是 Python。没有别的东西。

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

计数：**32** 个 `.py` 源文件，`bridge/` 中 **15** 个 `.py` 文件（14 个模块 + `__init__.py`），**10** 个测试模块，**2** 个示例驱动，**2** 个运行脚本。

**不**在那里的东西：

| 语言 | adapter 或 SDK | 仓库中的行数 |
|---|---|---|
| Go | 无 | **0** |
| C++ | 无 | **0** |
| Rust（通用） | 无 —— `Behavior` 内部存在但未暴露 | 外部 0 |
| Lua | 无（见 WM-0610） | 0 |
| BigWorld | **无 adapter、无 bridge、无模块** | 0 |

全仓库唯一的 Go 源码是 `tools/hdr_conformance/go/main.go`，它是一个一致性度量工具 —— 不是 SDK，也不依赖 controller。在 `engine-adapters/` 或任何语言 SDK 路径下不存在任何 `.go`、`.cpp`、`.cc`、`.hpp`、`.h` 或 `go.mod`。

BigWorld 是第四个缺席的语言面，值得单独点名，因为 `docs/tech-stack.md:16` 把它标为"待验证"而非缺席：字符串 "BigWorld" 在整个仓库中只出现在 `docs/roadmap.md:274`、`docs/tech-stack.md:16`、`docs/basin.md:114` 和 `wm-basin/src/overhead.rs:17`。没有代码，没有一致性 harness，没有能力目录。它的状态是诚实的：未定，决策推迟到实际出现的那个 SDK 和测试环境。

## 成文的语言立场（比较前先读）

| 语言 | `docs/tech-stack.md` | `docs/decisions.md` |
|---|---|---|
| KBEngine Bridge | "引擎脚本环境要求" | "Python —— KBEngine Bridge 所必需" |
| BigWorld Bridge | "按实际 SDK / Bot 环境决定 —— 待验证" |（未列出）|
| Python（通用） | "后续提供" | "通用 SDK 后续提供" |
| Lua | "v1 暂不实现" | "v1 暂不实现" |
| C++ | "独立进程或隔离 shim" | "原生 SDK / 独立进程 / 隔离 shim" |
| Go | "后续提供，不进入核心" | "非核心；将来可能有 SDK" |

其中两行对本评估起承重作用。第一，Python 行被刻意拆开：**bridge** 因引擎脚本环境的强制而*现在*就必需，而**通用 SDK** 是"后续"。这是两种不同的工件，盘点不得把它们混为一谈。第二，Go 以两种不同的表述被描述 —— 技术栈里是"后续提供"，决策里是"非核心；将来可能" —— 这更接近"被降级推迟"而不是"已列入计划"。下面的推荐尊重这个区分，而不是把它抹平。

## SDK 实际会包装什么

这里重要的发现是：**还没有任何 SDK —— 只有一个 bridge、一种语言、一个引擎。** 因此完备性问题主要是"哪个面已经准备好被包装"的问题。

第三方 SDK 可能坐落于其上的既有表面：

| 表面 | 位置 | 形状 |
|---|---|---|
| Controller REST | `crates/wm-controller/src/http.rs:47-52` | 5 个端点：`/healthz`、`/api/v1/status`、`/api/v1/runs`（POST）、`/api/v1/runs/{run_id}`（GET）、`/api/v1/runs/{run_id}/stop`（POST） |
| Control gRPC | `proto/control/v1/control_service.proto` | `ControlService` —— 2 个 RPC：`RegisterWorker`、`Heartbeat` |
| Backend gRPC | `proto/backend/v1/backend_service.proto` | `BackendService` —— 4 个 RPC：`Handshake`、`ApplyPlan`、`Start`、`Stop` |
| Protobuf 契约 | `proto/{common,control,backend,engine,metrics}/v1/*.proto` | 5 个文件 |
| `ExecutionBackend` | `docs/backends.md:13-23` | 11 个概念操作：`handshake`、`initialize`、`apply_plan`、`start`、`update_load`、`stop_admission`、`drain`、`shutdown`、`get_status`、`get_metrics`、`subscribe_events` |
| 原生 `Behavior` | `crates/wm-runtime/src/behavior.rs:100` | 一个方法，`Box<dyn Behavior>`；无注册表，无工厂（WM-0610 的发现） |

其中两行**不是** SDK 表面，而这个区分很重要：

- `ExecutionBackend` 是一个*概念*接口。`docs/backends.md:26` 明确说 "backend 不被要求以相同的 RPC 方法来实现它们"，且协议必须"显式描述不支持的能力"。它是审计一个 adapter 用的标尺，不是可实现的契约。
- `BackendService` gRPC（4 个 RPC）覆盖 handshake/plan/start/stop，但 `docs/backends.md:18-19` 中的 `StopAdmission` 和 `Drain` 没有对应的 RPC。`ExecutionBackend` 因此是 11 个操作只暴露出 4 个。这个缺口是结构性的，不是偶然的。

一个第三方编写型 SDK —— 让外部用户描述一个负载模式并读回结果的 SDK —— 会包装 **5 个 REST 端点和 plan-as-data**，而不是引擎 bridge。这正是 `docs/tech-stack.md:17-20` 中 "Python SDK / Lua SDK / C++ Adapter / Go SDK" 各行所指的表面。它在任何语言中都不存在。

## 完备性矩阵

以五个成文的语言面为评分对象，以 `docs/backends.md` 的 11 操作 `ExecutionBackend` 为标尺。单元格取值：**exists**（代码存在且经测试）、**partial**（存在但对照标尺不完整）、**absent**（无代码）、**deferred**（明确不在范围内）。

| 能力 | Python | Go | C++ | Lua | 原生 Rust |
|---|---|---|---|---|---|
| 引擎 bridge（引擎侧脚本） | **exists** —— `bridge/` 中 15 个 `.py` 文件、10 个测试模块、2 个 e2e 示例 | absent | absent | absent | n/a —— 原生 runtime 没有 bridge |
| 握手 + 能力握手 | **exists** —— `handshake.py` + `capability.py`，诚实的 `supported/unsupported/unknown/degraded` 目录 | absent | absent | absent | absent |
| `apply_plan`（plan-as-data → 引擎） | **exists** —— `plans.py` | absent | absent | absent | absent |
| `start` / `stop` | **exists** —— `commands.py` 分发 | absent | absent | absent | absent |
| `stop_admission` | **absent** —— 无 RPC，无 bridge 模块 | absent | absent | absent | absent |
| `drain` | **absent** —— 无 RPC，无 bridge 模块 | absent | absent | absent | absent |
| `update_load` | **absent** | absent | absent | absent | absent |
| `get_metrics`（bot 级） | **partial** —— 进程级存在（`process_metrics.py`）；bot 级在目录中记为 `unknown` | absent | absent | absent | absent |
| `subscribe_events` | **partial** —— `event_stream.py` 存在；每 bot 事件为 `unknown` | absent | absent | absent | absent |
| IPC / transport | **exists** —— `ipc_client.py`，带长度前缀的 UDS/TCP | absent | absent | absent | absent |
| 一致性 harness | **exists** —— `conformance/harness.py` | absent | absent | absent | absent |
| 通用第三方 SDK（计划编写 + Run 读回） | **absent** | absent | absent | absent | absent |
| 行为注册表（任何脚本化 SDK 都需要） | **absent** —— WM-0610 的发现 | absent | absent | absent | absent |

唯一有内容的列是 Python，而连它也只是 partial：11 个 `ExecutionBackend` 操作中有 4 个直接缺失（`stop_admission`、`drain`、`update_load`，以及 bot 级 `get_metrics`/`subscribe_events` 的拆分），因为这些操作在 `BackendService` 上没有 RPC，在引擎侧也没有 bridge 模块。

### 诚实地读这张矩阵

空单元格与其说是 Go 或 C++ 列的完备性欠缺，不如说是在陈述 **Go 和 C++ 列从未开始过**。Python 列是唯一尝试过东西的列，而它的缺口是对照概念标尺的缺口，不是对照可用代码的缺口。没有任何语言拥有通用第三方 SDK。因此"SDK 完备性"这个前提在评估之时就部分不成立：只有一种语言里的一个 bridge，任何地方都没有 SDK。

## 缺口清单

按每一项对*可用*外部 SDK 的阻塞程度排序，而不是按其规模：

1. **没有行为注册表。** `BatchRunner::spawn`（`crates/wm-runtime/src/batch.rs:89`）直接拿一个 `Box<dyn Behavior>`。在任何语言里都没有办法*叫出*一个行为的名字。这个门槛与 WM-0610 共享，适用于矩阵中每一个脚本化 SDK 行。
2. **`BackendService` 只暴露 11 个 `ExecutionBackend` 操作中的 4 个。** `StopAdmission` 和 `Drain` 没有对应 RPC（`docs/backends.md:18-19`）。任何语言的 SDK 今天都无法做到尊重优雅停机。
3. **`update_load` 完全没有任何表面。** 它出现在概念接口里，但 gRPC 服务和 bridge 里都没有。
4. **bot 级可观测性在 KBEngine 目录中是 `unknown`。** `capability.py` 把 `connected_count`、`login_success` 和 `per_bot_events` 记为 `unknown`，把 `message_latency` 记为 `unsupported` —— 这很诚实，但意味着 Go 或 C++ SDK 无法承诺 Python bridge 已经声明不提供的内容。
5. **任何地方都没有 Go 和 C++ 代码。** 没有 `go.mod`，没有头文件，没有构建。`docs/tech-stack.md:19-20` 那两行是前瞻性的，不是进行中的。
6. **BigWorld 是未定，不是缺席。** `docs/roadmap.md:274` 把它放在 "v1 Optional" 之下，带限定语"取决于 SDK 和测试环境"。这个限定语仍然悬而未决 —— 没有任何源码审阅，也没有对照任何 BigWorld 版本的度量（`docs/basin.md:114` 正要求先做到这一点才能做任何能力声明）。
7. **plan-as-data 没有类型化客户端。** 5 个 REST 端点收发 JSON；没有任何语言绑定从它们生成。这才是*第三方* SDK 真正的阻塞缺口，而且它与先选哪种语言正交。
8. **adapter 自己的 README 只记录了它自己的一半。** `engine-adapters/kbengine/README.md`（86 行）有一个 `Layout` 块，只点名了 **14 个 bridge 模块中的 5 个** —— `bootstrap`、`capability`、`command_queue`、`engine_port`、`ipc_client` —— 漏掉了 `bot_registry`、`commands`、`connection_events`、`event_stream`、`handshake`、`lifecycle`、`metrics`、`plans` 和 `process_metrics`。它的 `tests/` 子树只列出 `test_bootstrap` 和 `test_ipc_client`，而实际存在 **10** 个测试模块；并且该块的逐模块注释仍把 `WM-0201` / `WM-0202` 引为那些文件中最新的工作 —— 而树中的模块 docstring 引用的任务已到 **`WM-0216`**（`plans.py`、`connection_events.py`），单是 `bootstrap.py` 就横跨 `WM-0201` → `WM-0215`。精确地说：README 的*正文*是最新的（84-86 行引用了 `WM-0216` / `WM-0217`），所以这不是一份过时的文档 —— 它是一张**过时的图**。权威的 `Layout` 块是新来的人最先读的东西，它描绘的是树的更早状态，是完备性评审应当标记出来刷新的那个具体工件。

## 推荐

**先构建 Python SDK，然后 Go，C++ 推后 —— 因为 Python bridge 是树中唯一已经证明了该模式的工件，而它里面的每一个缺口都可以在不接触新引擎的情况下修复。**

优先级顺序，以及每一项排在那个位置的原因：

| # | 目标 | 理由 |
|---|---|---|
| 1 | **Python 通用 SDK** | 唯一拥有任何生产代码的语言（`bridge/` 有 14 个模块加 `__init__.py`，另有 10 个测试模块和 2 个 e2e 驱动）。KBEngine 路径已经演示了握手、能力协商、成帧、命令分发和 plan 应用 —— 所以 SDK 是一次*追加*（经 REST 的计划编写 + Run 读回），而不是从零开始。`docs/tech-stack.md:17` 已经写着"后续提供"；这就是"后续"的含义。 |
| 2 | **Go SDK** | 第二，依据是 `docs/decisions.md:22`（"将来可能有 SDK"）以及仓库已经携带 Go 工具（`tools/hdr_conformance/go/`）这一事实，所以工具链并不陌生。包装同样那 5 个 REST 端点。`docs/tech-stack.md:20`（"不进入核心"）把它降了级 —— 所以它跟在 Python 后面，而不是与 Python 赛跑。 |
| 3 | **C++ adapter** | 最后，且只在存在具体需求时。`docs/tech-stack.md:19` 写的是"按需"，`docs/decisions.md:21` 写的是"独立进程或隔离 shim" —— 那是一条*设计方针*，不是承诺。今天范围内的引擎没有一个需要它，而且隔离 shim 的形态意味着它会是一条*全新*的进程边界，而不是进程内绑定。等某个具名的 bot 框架提出要求时再建。 |
| — | **Lua** | 推迟 —— 由 WM-0610（`docs/evaluation/lua-behavior-sdk.md`）单独决定：已选 mlua，未动工，被同一个行为注册表卡住。 |
| — | **BigWorld** | **未定。** 不是优先级项。`docs/tech-stack.md:16` 把它标为"待验证"，`docs/basin.md:114` 要求在任何能力声明之前先做源码审阅加上在目标版本上的度量。在实际的 BigWorld SDK 和测试环境被确认之前，这一行按设计保持开放。 |

### 无论选哪种语言，先解除什么阻塞

阻塞性的工作与语言无关：

1. 一个 runner 消费的**行为注册表** —— 任何脚本化 SDK 都需要，Rust 或其他（WM-0610 缺口 #1，共享）。
2. **从 5 个 REST 端点和 5 个 proto 文件生成的类型化绑定** —— 这才是"SDK"对一个第三方具体意味着的东西，它对每一列同等阻塞。
3. **`StopAdmission` 和 `Drain` 的 RPC**，如果优雅停机要从 Rust 核心之外得到尊重。

### 本推荐改变哪些决策

- `docs/tech-stack.md:17-20` 获得一个优先级排序，而不是四条平行的"后续/按需"行。
- `docs/roadmap.md:274` 的 BigWorld 行保持显式推迟，条件写明而不是暗示。
- `docs/backends.md` 的 `ExecutionBackend` 仍是*概念性*标尺。两个缺失的 RPC 在这里被记录为缺口，而不是被无声地吸收进"协议"。

## 注意事项

1. **这张矩阵是时点盘点，不是预测。** 它反映 2026-11-12 的 `engine-adapters/`；`bridge/` 里新增一个模块就会改变一行。
2. **BigWorld 没有已核实的事实。** 这里关于它的每一句声明都是"尚未确立"，而这本身就是发现。`docs/basin.md:114` 禁止把未经度量的假设提升为 adapter 能力，本页尊重这条规则。
3. **`ExecutionBackend` 不是可实现接口。** 它自我描述为概念性契约（`docs/backends.md:26`）。对照它度量出的完备性是*缺口度量*，不是合规测试。
4. **仓库里的 "Go" 不是 SDK。** `tools/hdr_conformance/go/main.go` 是一个基准测试工具。把它算作 Go SDK 进展会是错误的；本评估不这样算。
5. **没有任何语言拥有第三方 SDK。** 这是最清晰的单条发现。矩阵的空单元格是准确的，不应被读作对缺席语言的指控 —— 它们是对尚未开始之事的描述。

## 延伸阅读

- `docs/backends.md:13-23` —— 本页用作标尺的 11 操作 `ExecutionBackend`
- `engine-adapters/kbengine/README.md` —— 布局、配置注入、线上格式，以及被遵守的三条引擎约束
- `engine-adapters/kbengine/bridge/capability.py` —— 本矩阵所依赖的诚实 `supported / unsupported / unknown / degraded` 目录
- `crates/wm-controller/src/http.rs:47-52` —— SDK 将要包装的 5 个 REST 端点
- `docs/evaluation/lua-behavior-sdk.md` —— Lua 行的配套评估，包括共享的行为注册表门槛

## 结论

**一个 bridge，一种语言，没有 SDK。** `engine-adapters/` 里有且只有一个 adapter —— Python 的 KBEngine bridge —— Go、C++、Lua 以及 BigWorld 方向什么都没有。诚实的完备性解读是：Python 对照 `ExecutionBackend` 是 *partial*（11 个操作缺 4 个，主要因为根本没有 RPC），Go、C++ 和 Lua 则完全 *absent*，BigWorld 按设计 *undetermined*。

先构建 Python SDK，然后 Go，C++ 推迟到某个具名的 bot 框架提出要求。在选择语言之前先解除共享前提的阻塞：行为注册表、从既有 REST 与 proto 契约生成的类型化绑定，以及 `StopAdmission` 和 `Drain` 的 RPC。BigWorld 保持开放，直到有人审阅其源码并对它做度量 —— 这才是正确的状态，不是一条要按日程关闭的缺口。
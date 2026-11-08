# ADR-005: Execution Backend Model

> 状态：Accepted（部分保留：Mode 定义待原始决策文档确认）
> 来源：计划书 §0.1 / §4 / §6.6 / §23。WM-0001 的部分闭合材料。
> 注：计划书引用的 `wavemaker-stack-decision.md` 不在本地，凡涉及该文档原始结论处均标注「待原始决策文档确认」。

## Context（背景）

Wavemaker 需要同时支持自研高性能执行与复用游戏引擎原生 Bot 两类负载生成方式。原始架构文档（不在本地，待原始决策文档确认）定义了三种接入模式 Mode A/B/C，计划书 §6.6 要求将其统一到 Backend 模型下。

## Decision（决策）

### D1：双 Backend 类型 + 混合执行（计划书 §0.1.3、§4）

v1 交付两种 Execution Backend，均实现统一的 `ExecutionBackend` 概念接口（§4.1：handshake / initialize / apply_plan / start / update_load / stop_admission / drain / shutdown / get_status / get_metrics / subscribe_events）：

1. **Native Runtime Backend**：Rust Worker，由 Wavemaker 自己运行 Actor、连接、协议与行为（§4.2）。Actor API 不暴露给外部引擎 Bridge。
2. **Engine Bot Backend**：Supervisor + 引擎原生 Bot + Bridge（§4.3）。引擎拥有其内部 Bot 生命周期；Bridge 将引擎可见的控制能力映射为 Backend API；对不能实现的操作返回明确的 Unsupported / CapabilityUnavailable，**不得伪造单 Bot 控制能力**。

一个 Swell 可混合多 Backend（§4.4），`weight` 仅为分配示意；实际计划必须展开为确定的 Backend 分配与 ShardPlan，不得在 Worker 端随机分配。

### D2：Mode A/B/C → Backend 模型映射（计划书 §6.6）

| Mode | 含义（计划书 §6.6 原文） | Backend 模型映射 |
|---|---|---|
| Mode A | 引擎托管 Bot；Wavemaker 通过 Bridge 控制和观测原生 Bot | Engine Bot Backend（v1 落地为 KBEngine adapter） |
| Mode B | Wavemaker 管理执行器，但复用引擎提供的部分客户端能力；具体边界按现有架构定义，不得假设拥有 socket | **定义已冻结（WM-0001，docs/architecture.md「Mode A/B/C Definitions」）**：socket 归引擎 client 库、执行器进程归 Wavemaker，控制粒度为逐会话。v1 不实现；实现前需单独 ADR 显式声明能力边界 |
| Mode C | Wavemaker Native Runtime 自己持有连接和协议执行 | Native Runtime Backend |

三种模式不是同一套 Actor 的不同编译选项，而是不同的执行后端/接入能力组合（§6.6）。若后续重新定义 Mode，需单独记录 ADR。

### D3：Backend 状态机（计划书 §4.1.1）

```text
Created -> Handshaking -> Ready -> Initialized -> PlanAccepted -> Running
  -> AdmissionStopped -> Draining -> Stopped
任意运行态 -> Degraded / Failed
Failed -> Recovering -> Handshaking
```

每次状态变化必须携带 backend_id、swell_id、state、observed_at、reason_code、detail、generation/epoch；不得仅靠 Controller 命令发送成功推断 Backend 已完成状态转换。

### D4：KBEngine 为第一款 Engine Bot Backend（计划书 §6.1）

选择理由（计划书原文）：已有原生 Bots；C++ 网络运行时与 Python 行为脚本边界明确；可复用已有 Bot 连接和协议实现；有 Python 脚本扩展入口适合实现 Bridge；无需把 C++ client_lib 链接进 Rust Worker。KBEngine API 能力以 conformance test / 源码审查为准（WM-0008/0009，未完成，保持 Unknown）。

## Consequences（后果）

- 混合执行必须在报告中分开呈现 Backend 结果，记录每种 Backend 的能力和调度精度，不把不同客户端实现的结果默认视为严格等价（§4.4）。
- 容器化与桥接正交：Supervisor 管进程/容器生命周期，Bridge 管控制/事件/指标交换（§0.1.5）。v1 只要求 NativeProcessLauncher（§7.1）。
- 第三方 Backend 后续允许加入，但必须遵守统一协议与能力模型（§2.2）。
- Mode B 定义已冻结在 `docs/architecture.md`「Mode A/B/C Definitions」（WM-0001，2026-11-09）：引擎 client 库持有 socket、Wavemaker 持有执行器进程；v1 不实现，任何实现前必须以单独 ADR 声明能力边界，不得宣称覆盖未兑现的观测语义。

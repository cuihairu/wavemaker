---
title: 总体架构
---

# 总体架构

## 系统架构

<ArchitectureDiagram />

## 2.1 逻辑架构

```text
                         Web Dashboard / CLI
                                  |
                           HTTP / WebSocket
                                  |
                       +----------------------+
                       | Wavemaker Controller |
                       | Rust + Axum + Tokio  |
                       +----------------------+
                         |        |         |
                  Plan API   Backend API  Report API
                         |        |         |
                  +------+--------+---------+------+
                  |          Control Protocol      |
                  |             Protobuf           |
                  +------+-------------------+------+
                         |                   |
             +-----------+----+      +-------+----------------+
             | Native Worker  |      | Engine Bot Supervisor  |
             | Rust + Tokio   |      | Rust                    |
             +----------------+      +-------------------------+
             | Actor Runtime  |                 |
             | Scheduler      |          Local IPC / RPC
             | Virtual Clock  |                 |
             | Transport      |       +---------+-----------+
             | Protocol       |       | Engine Bot Process  |
             | Metrics        |       | KBEngine Bots       |
             +----------------+       | C++ Runtime         |
                                      | Python Bridge       |
                                      +---------------------+

            +-------------------- Basin ----------------------+
            | Topology / Providers / Metric Mapping / Queries |
            | Prometheus / OTel / Custom Provider             |
            +-------------------------------------------------+
                                  |
                         Swell Report Pipeline
```

## 2.2 控制面与执行面

### Control Plane

Rust Controller 负责：

- Swell 生命周期。
- Spectrum 校验。
- 全局计划生成。
- ShardPlan 生成与冻结。
- Backend 注册与能力协商。
- Backend 分配和租约。
- 控制命令与状态同步。
- Basin 配置和观测任务编排。
- 数据完整性检查。
- 报告生成任务编排。

### Execution Plane

由不同 Backend 实现：

- Native Runtime Backend：Rust Worker。
- Engine Bot Backend：Supervisor + 引擎原生 Bot + Bridge。
- 后续允许第三方 Backend，但必须遵守统一协议和能力模型。

### Observability Plane

分为三类数据：

1. Generator：Worker、Supervisor、Bot 进程和 Bot 状态。
2. Server：Basin 中的服务端主机、进程、服务、数据库和业务指标。
3. Experiment：计划负载、实际到达、阶段、故障事件、部署版本和时间范围。

---


## 1.3 三类执行实体

必须严格区分：

1. **Controller**：管理 Swell、生成计划、调度 Backend、汇总数据。
2. **Worker / Backend**：实际执行负载的管理边界。
3. **Actor / Engine Bot**：具体虚拟客户端。

对于 Native Runtime，一个 Worker 可管理数千个 Actor。

对于 KBEngine，一个 Engine Bot Backend 可管理一个或多个 KBEngine Bots 进程，每个进程内部由 KBEngine 管理多个 Bot。

Controller 管 Backend；Backend 管进程和其可见的 Bot；引擎管理 Bot 内部状态。

## 1.4 Mode A / Mode B / Mode C 定义（WM-0001）

> 状态：**v1 冻结** ——本节是三种接入模式在仓库内的权威定义。最初提出这些模式的原始架构文档不在本地（ADR-005 已注明）；以下定义依据计划书 §6.6 表述与 KBEngine 源码审计事实（`docs/spec/kbengine-api-audit.md`）钉死，每条能力主张都可回溯到源码证据。若原始文档日后出现且不一致，以新 ADR 调和。

三种模式不是同一套 Actor 的编译选项。每种模式都是「执行后端 + 接入能力」的组合，由三个所有权问题区分：谁创建 Bot、谁持有 socket、Wavemaker 能以什么粒度控制和观测。

| 问题 | Mode A——引擎托管 Bot | Mode B——引擎客户端栈 + Wavemaker 执行器 | Mode C——Native Runtime |
|---|---|---|---|
| Execution Backend | Engine Bot Backend | Engine Bot Backend（能力变体） | Native Runtime Backend |
| Bot 创建 | 引擎原生创建 | Wavemaker 执行器创建会话 | Wavemaker Worker 创建 Actor |
| Socket / 连接所有权 | 引擎 C++ 运行时 | 引擎 client 库（由 Wavemaker 执行器进程驱动） | Wavemaker Worker |
| 控制粒度 | 仅批次级 | 逐会话命令 | 逐 Actor 命令 |
| 观测 | 引擎暴露的聚合与事件 | client 库透出的会话事件 | 完整逐 Actor 状态 |
| 行为代码 | 引擎脚本 | 经 client 库 API 由 Wavemaker 驱动 | Wavemaker Behavior（v1 为 Rust） |
| v1 可用性 | 首个目标（KBEngine adapter，Phase 2） | v1 不实现；实现前必须单独 ADR | Phase 3 |

Mode A 的 KBEngine 事实（证据见 `docs/spec/kbengine-api-audit.md`）：

- Bot 创建只有批次语义且为累加：`addBots` 累加全局待创建计数，无返回值、无失败回调（`bots.cpp:344-388`）；Bridge 必须在 StopAdmission 前自建批次预算（审计 §1，差异项 D1/D2）。
- 不存在单 Bot 创建 API：`KBEngine.bots` 是以 int32 client appID 为键的只读映射（`pybots.cpp:16-113`）；单 Bot 控制不可用，逐 Bot 观测必须走各 `ClientObject`（审计 §2）。
- 逐 Bot 延迟不能从进程级数据推导，总数不得冒充在线用户——能力主张以审计过的 API 为准（「No Unverified Capability Claims」）。

Mode B 在此冻结定义但 v1 不实现：socket 归引擎 client 库，执行器进程归 Wavemaker，因此具备 Mode A 缺失的逐会话控制。未来的 Mode B Backend 必须在自己的 ADR 中显式声明能力边界，不得主张其无法兑现的观测语义。

Mode C 即 Native Runtime Backend：Worker 自己持有连接、协议执行与行为调度（`docs/native-runtime.md`）。它是唯一由 Wavemaker 完全掌控逐 Actor 到达精度的模式。

后果：

- 能力协商按 Backend 上报模式；Bridge 只能返回 Unsupported，不得用粗粒度模式冒充更细粒度模式。
- 混合 Swell 的报告按模式分组（ADR-005）。

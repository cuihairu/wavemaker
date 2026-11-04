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

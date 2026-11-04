---
title: 最终技术决策摘要
---

# 最终技术决策摘要

| 决策 | 结果 |
|---|---|
| 核心语言 | Rust |
| Controller / Web | Rust + Axum |
| Worker | Rust + Tokio |
| Supervisor | Rust |
| 跨进程契约 | Protobuf |
| KBEngine 接入 | Python Bridge + Rust Supervisor |
| 原生 Bot 复用 | 一等 Execution Backend |
| 容器 | 可选部署方式 |
| Native Runtime | Wavemaker 自有 Actor Runtime |
| Behavior v1 | Rust 内部 Behavior |
| Lua | v1 不实现 |
| Python | KBEngine Bridge 必需；通用 SDK 后续 |
| C++ | 原生 SDK / 独立进程 / 隔离 shim |
| Go | 非核心，未来可提供 SDK |
| 服务端监控 | Basin |
| 发生器监控 | Generator Observability |
| 计划生成 | Controller 唯一生成，plan-as-data |
| 分布式 | ShardPlan 冻结、租约、generation |
| 报告 | 计划、实际负载、发生器、服务端、数据质量统一呈现 |

**最终原则：Wavemaker 统一的是压测管理、计划、控制协议、观测和报告，不是强迫所有游戏引擎使用同一种 Bot 实现。**

引擎可以继续使用自己的原生 Bot、网络栈和脚本系统；Wavemaker 通过 Backend + Supervisor + Bridge 将它们纳入统一实验管理。与此同时，Wavemaker 自己仍保留高性能 Rust Native Runtime，服务于没有成熟原生 Bot 或需要精确控制的引擎。

这才是跨引擎压测框架应有的兼容边界。

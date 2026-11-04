---
title: 项目定位
---

## 1.1 项目定位

Wavemaker 是一个**跨引擎游戏服务器负载测试框架**。它通过统一的负载模型、计划数据、执行后端接口和观测模型，生成、管理并分析大量虚拟客户端对游戏服务端施加的负载。

它不是单纯的 Bot 脚本框架，也不是通用分布式任务队列。

核心价值：

- 使用统一 Spectrum 描述压测负载。
- 支持自有 Rust Actor Runtime。
- 复用已有游戏引擎的原生 Bot。
- 在单机和多机环境中管理执行后端。
- 同时观测 Generator 与被测 Server。
- 输出包含数据质量、负载偏差和服务端曲线的可审计报告。


## 1.2 领域术语

| 名称 | 定义 | 不应混淆为 |
|---|---|---|
| Wavemaker | 整个压测框架 | 单个 Worker |
| Swell | 一次完整压测实验及其生命周期 | 单个任务命令 |
| Spectrum | 描述负载形状、到达过程、行为分布和生命周期的配置 | 单个 Bot 脚本 |
| Basin | 被测环境及其服务拓扑、服务端观测配置 | Prometheus 本身 |
| Controller | 控制面，负责计划生成、任务管理、分配和报告编排 | Actor 执行器 |
| Worker | Wavemaker 管理的执行节点 | 必然等同于一个 Actor |
| Execution Backend | 实际生成负载的执行实现 | 进程管理器 |
| Native Runtime Backend | Wavemaker 自己的 Rust 执行后端 | 游戏引擎原生 Bot |
| Engine Bot Backend | 复用引擎自带 Bot 的执行后端 | Wavemaker Actor |
| Supervisor | 管理 Backend 进程/容器生命周期的组件 | 游戏引擎内部调度器 |
| Bridge | 引擎侧与 Wavemaker 之间的控制/事件/指标适配层 | 仅仅是网络代理 |
| Actor | Native Runtime 中的虚拟客户端 | 所有引擎原生 Bot 的统一对象 |
| ShardPlan | Controller 生成并冻结的执行计划分片 | Worker 自行生成的计划 |
| Generator Observability | 压测发生器自身的资源和运行状态观测 | Basin 服务端监控 |
| Server Observability | 被测服务端的指标采集和时间序列 | Generator 指标 |

---
title: 测试与验证策略
---

# 测试与验证策略

## 16.1 测试层级

### Unit

- Spectrum 校验。
- Plan 生成。
- ShardPlan 冻结。
- Actor 状态机。
- Backend 状态机。
- Bridge 消息映射。
- 指标聚合。
- Histogram 编解码。

### Contract / Conformance

- Rust Controller ↔ Rust Worker。
- Rust Supervisor ↔ Python Bridge。
- 不同语言 Protobuf codegen 一致性。
- 事件序列与重复事件处理。
- 能力声明与实际行为一致。
- HDR V2 blob 跨实现合并一致。

### Integration

- KBEngine Bots 启动和 Bridge 注册。
- 批次创建。
- 登录事件采集。
- 停止新增。
- 优雅退出。
- 断线重连。
- Supervisor / Bridge / Bot 进程故障。

### Load / Soak

- 5k / 20k / 50k Actor。
- 每 Actor 4 个定时器。
- 事件突发。
- 大量连接建立和关闭。
- 24h soak。
- Bridge 事件上报压力。
- 监控 Provider 高延迟。
- Controller 重启和 Backend 重新注册。

## 16.2 性能裁决实验

ADR 中 Rust / Go / Python 的性能论证应通过真实基准验证，而不是只比较数据结构复杂度。

至少测试：

1. Tokio timer insert/cancel/expire。
2. Go timer heap 对照。
3. Python heapq / uvloop 对照（只用于适用模式）。
4. 5k / 20k / 50k Actor 的调度 P50/P95/P99。
5. 每 Actor 4 个 timer 的内存。
6. 创建速率 1k–5k/s 时 planned_at 偏差。
7. Bridge IPC 吞吐、延迟、批次大小和队列增长。
8. KBEngine callback 的调度粒度和积压行为。
9. Supervisor 进程监控开销。
10. Generator Metrics 对运行时的开销。

## 16.3 测试有效性

一次 Swell 只有满足以下条件才能标记为 Valid：

- 计划配置通过校验。
- 计划摘要与 Backend 接收摘要一致。
- Backend 状态和实际负载数据完整。
- 发生器没有达到不可接受的饱和状态。
- 必需观测数据满足覆盖要求，或明确标记 Partial。
- 时间序列没有无法解释的大段缺失。
- 计划到达和实际到达偏差在用户定义阈值内。
- 没有未处理的关键故障。

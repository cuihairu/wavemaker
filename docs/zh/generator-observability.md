---
title: Generator Observability 与数据质量
---

# Generator Observability 与数据质量

## 11.1 必须监控发生器自身

每个 Worker / Supervisor / Engine Bot Process 需要记录：

- CPU、RSS、线程数、FD。
- 事件队列长度。
- 控制通道延迟。
- 事件上报丢失数。
- 指标上报延迟。
- planned_at 与 actual_at 偏差。
- Scheduler lag。
- Actor / Bot 数量。
- 活跃连接数。
- 进程重启和失联事件。

发生器达到资源瓶颈时，报告必须标记 Generator Saturated，不能将发生器瓶颈误判为服务端瓶颈。

## 11.2 HDR Histogram

- 所有 Worker 使用统一 sigfigs、lowest、highest 和 epoch 定义。
- 参数由 Controller 下发并写入计划元数据。
- V2 histoblob 作为 opaque bytes 传输时，必须携带 schema/version/参数摘要。
- 不允许直接平均各 Worker 的 P99。
- 使用可合并直方图汇总总体分布。
- coordinated omission 修正在 Worker 侧进行。
- 跨语言实现必须通过同输入、同参数、同输出的 conformance test。

## 11.3 数据质量标签

报告必须能显示：

- Complete
- Partial
- Delayed
- Missing
- Estimated
- Invalid
- GeneratorSaturated
- ObserverDegraded

缺失数据不能默认为零。

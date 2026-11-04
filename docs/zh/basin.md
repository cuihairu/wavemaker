---
title: 服务端观测：Basin
---

# 服务端观测：Basin

## 10.1 Basin 定位

Basin 表示被测环境及其服务拓扑和观测配置，不是监控产品，也不负责重新实现 Prometheus。

```text
Basin
  ├── Environment
  ├── Topology
  ├── Observability
  │     ├── Provider Registry
  │     ├── Metric Catalog
  │     ├── Metric Mapping
  │     ├── Query Scheduler
  │     ├── Time Alignment
  │     └── Data Quality
  └── Correlation Context
```

## 10.2 Provider

v1 建议：

- Prometheus Provider：优先实现。
- OpenTelemetry Provider：按实际数据接入需求。
- Custom Provider：保留统一接口。
- Grafana：作为现有监控可视化/数据源入口，不作为指标存储语义的替代品。

## 10.3 指标类别

### Host

- CPU、内存、磁盘 IO、网络吞吐、丢包。

### Process

- CPU、RSS、线程数、FD、GC（如适用）、事件循环延迟。

### Gateway / Login

- 连接数、连接建立速率、消息吞吐、登录成功率和延迟。

### Game

- 在线人数、Tick duration、Tick overrun、消息处理时间。

### Queue / RPC

- 队列长度、等待时间、RPC 次数、成功率和延迟。

### Database / Cache

- 查询速率、查询延迟、连接池、命中率和错误率。

### Business

- 匹配耗时、房间数、场景人数、关键业务操作成功率。

指标必须有统一单位和语义目录，不能只依赖 Prometheus 指标名称猜测含义。

## 10.4 时间戳模型

至少保留：

- source_timestamp：源系统产生样本的时间。
- observed_at：Wavemaker 认为该样本对应的观测时间。
- collected_at：Wavemaker 实际收集到数据的时间。

还应记录：

- 查询时间窗口。
- scrape / collection interval。
- 数据源延迟。
- 缺失区间。
- 时钟偏移估计。
- 数据源实例和标签。

不以 NTP 绝对同步作为唯一前提。可用负载曲线和事件形状进行辅助对齐，但形状对齐不能取代真实时间戳，也不能用于伪造时间精度。

## 10.5 Preflight

Swell 启动前检查：

- Provider 可连接。
- 必需指标存在。
- 目标拓扑覆盖完整。
- 指标单位和类型映射正确。
- 采样间隔满足报告需求。
- 查询窗口可用。
- 监控数据源权限足够。
- 是否存在高基数标签风险。

如果 Server Observability 不可用：

- 可以按配置允许继续压测。
- 报告标记 `Partial Observability`。
- 不得给出完整服务端性能结论。

## 10.6 观测开销

沿用 ADR 的三级观测思路：

- L0：低开销基础进程/系统指标，默认开启。
- L1：watcher 或引擎专用监控，启用前实测开销。
- L2：eBPF 等高开销诊断，仅限时窗口启用。

`observer_baseline` 空载采样 60 秒；如果观测开销超过目标阈值（ADR 当前提出 3%，需定义测量方法），自动降级并在报告中记录。

KBEngine / BigWorld 专用 watcher 的调用频率必须按实际实现验证。ADR 中关于 KBEngine `cluster_controller.py` 的 5 秒上限、BigWorld watcher 锁行为等结论，应在目标版本源码审查和实测后写入 adapter capability，不要作为所有版本的永久假设。

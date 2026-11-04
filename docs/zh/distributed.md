---
title: 分布式管理与故障处理
---

# 分布式管理与故障处理

## 12.1 Worker / Backend 注册

注册信息：

- worker_id
- backend_id
- host_id
- backend_type
- engine / runtime version
- adapter version
- protocol version
- capabilities
- resource capacity
- labels
- last_heartbeat
- generation

## 12.2 租约

沿用 ADR 当前参数作为初始候选：

- 心跳：1 秒。
- TTL：5 秒。
- 连续 3 次判定失联。

这些参数需在网络抖动和高负载条件下验证。

失联策略：

1. 冻结新增负载。
2. 不主动断开已有连接。
3. 记录租约失效时间。
4. 允许 Backend 按策略保持已有会话。
5. 恢复后重新握手并同步状态。
6. 超过最大失联时长后按 Swell 的 DrainPolicy 执行。
7. 旧 generation 的事件丢弃或归档，不得覆盖新状态。

## 12.3 故障分类

- Controller 失联。
- Worker 失联。
- Supervisor 崩溃。
- Engine Bot 进程崩溃。
- Bridge 断线。
- 监控 Provider 失联。
- 目标服务器不可用。
- 本机资源耗尽。
- 事件队列溢出。
- 计划数据不一致。

每种故障都要定义检测、状态、恢复动作、报告影响和是否允许自动重试。

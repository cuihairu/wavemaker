---
title: Engine Bot Bridge 设计
---

# Engine Bot Bridge 设计

## 5.1 设计目标

Bridge 的目标是让引擎原生 Bot 能够被 Wavemaker 管理和观测，而不是把引擎 Bot 改造成 Wavemaker Actor。

Bridge 应支持：

- 身份注册。
- 版本和能力协商。
- 初始化配置。
- 负载计划提交。
- 启动和停止新增。
- 优雅退出。
- 状态查询。
- Bot 生命周期事件。
- 业务事件。
- Generator 指标。
- 心跳与重新连接。
- 数据完整性和事件序列检查。

## 5.2 两种通信边界

### Controller ↔ Supervisor

建议使用 Tonic / gRPC + Protobuf：

- 控制命令。
- 状态查询。
- Backend 注册。
- 租约和心跳。
- 事件/指标批量上报。

### Supervisor ↔ Engine Bridge

优先本机 IPC：

- Linux：Unix Domain Socket。
- Windows：Named Pipe。
- TCP loopback：作为跨平台回退或调试方式。
- Stdio RPC：只作为开发调试方案，不作为默认生产通道。

Bridge 与 Supervisor 可以采用双向流，也可以采用请求/响应加事件流。必须先定义消息顺序、重连和幂等语义。

## 5.3 Bridge 协议草案

以下是逻辑协议草案，具体字段需在 Phase 0 通过 Protobuf 定义冻结。

```protobuf
syntax = "proto3";

package wavemaker.engine.v1;

service EngineBridge {
  rpc Handshake(HandshakeRequest) returns (HandshakeResponse);
  rpc Initialize(InitializeRequest) returns (OperationResult);
  rpc ApplyPlan(ApplyPlanRequest) returns (OperationResult);
  rpc Start(StartRequest) returns (OperationResult);
  rpc StopAdmission(StopAdmissionRequest) returns (OperationResult);
  rpc Drain(DrainRequest) returns (OperationResult);
  rpc Shutdown(ShutdownRequest) returns (OperationResult);
  rpc GetStatus(StatusRequest) returns (StatusResponse);
  rpc GetMetrics(MetricsRequest) returns (MetricsResponse);
  rpc SubscribeEvents(EventSubscription) returns (stream EngineEvent);
}
```

### 5.3.1 Handshake

返回：

- engine_name
- engine_version
- adapter_name
- adapter_version
- protocol_version
- supported_capabilities
- unsupported_capabilities
- metric_schema_version
- max_batch_size
- scheduling_resolution
- clock_source
- process_id（适用时）
- backend_instance_id

协议版本不兼容时必须拒绝启动，不能静默降级。

### 5.3.2 能力模型

能力值不能只有简单的 true/false。建议：

```text
Supported
Unsupported
Unknown
Degraded
```

建议能力目录：

```yaml
capabilities:
  process:
    start: supported
    stop: supported
    restart: supported
    resource_limits: supported

  bots:
    create_batch: supported
    create_individual: unknown
    destroy_individual: unknown
    pause_individual: unknown
    inspect_individual: unknown

  scheduling:
    exact_arrival: unsupported
    batch_arrival: supported
    resolution_ms: 1000

  observability:
    process_cpu: supported
    process_memory: supported
    bot_count: supported
    connected_count: unknown
    login_success: unknown
    per_bot_events: unknown
    message_latency: unsupported
```

能力声明必须由 adapter conformance test 生成或验证，不能只靠配置文件手写。

## 5.4 命令语义

| 命令 | 语义 | 幂等要求 |
|---|---|---|
| Handshake | 协议与能力协商 | 是 |
| Initialize | 初始化一个 Swell 的 Backend 实例 | 同一 generation 幂等 |
| ApplyPlan | 接收冻结的计划分片 | plan_digest 相同幂等 |
| Start | 开始执行已接受计划 | 同一 operation_id 幂等 |
| StopAdmission | 停止新增 Bot，不关闭已有连接 | 幂等 |
| Drain | 按策略等待或清理现有 Bot | operation_id 幂等 |
| Shutdown | 结束 Backend 实例 | 幂等 |
| GetStatus | 返回当前状态快照 | 是 |
| GetMetrics | 返回指定窗口指标快照 | 是 |
| SubscribeEvents | 订阅事件流 | 通过 cursor 恢复 |

不要把 `Stop` 同时解释为停止新增和断开已有连接。必须分成 `StopAdmission` 与 `Drain`。

## 5.5 事件模型

事件至少包括：

- BackendStarted
- BackendReady
- BotCreated
- BotConnected
- BotLoginSucceeded
- BotLoginFailed
- BotStateChanged
- BotDisconnected
- BotDestroyed
- BotError
- AdmissionStopped
- BackendDegraded
- BackendFailed

每个事件包含：

- event_id
- backend_id
- swell_id
- shard_id
- bot_id（如可获得）
- sequence
- source_timestamp
- observed_timestamp
- event_type
- payload
- schema_version

事件应支持批量发送、重连后从 cursor 继续、重复事件去重。若引擎无法提供稳定 Bot ID，则必须定义 adapter 生成的稳定映射 ID，并注明其生命周期范围。

## 5.6 指标模型

至少区分：

### 进程级

- CPU usage
- RSS / virtual memory
- thread count
- file descriptor / handle count
- process uptime
- event loop / callback lag
- IPC queue depth
- dropped events

### Bot 级聚合

- total bots
- created bots
- connected bots
- logged-in bots
- failed bots
- disconnected bots
- pending bots
- bots by state

### 业务级

- login attempts / success / failure
- message count
- message bytes
- operation success / failure
- operation latency（只有能可靠测量时才提供）
- reconnect count

每个指标必须定义：

- 名称和描述。
- 单位。
- Counter / Gauge / Histogram。
- 聚合语义。
- 采样间隔。
- 数据来源。
- 是否为引擎原生指标或 Bridge 推导指标。
- 统计误差和缺失语义。

## 5.7 Bridge 线程模型

不要从 RPC 线程直接调用引擎对象。

推荐模型：

```text
IPC Receiver
    |
    v
Command Queue
    |
    v
Engine Main / Callback Thread
    |
    +--> invoke engine API
    +--> inspect bot state
    +--> collect engine events
    |
    v
Event / Metrics Queue
    |
    v
IPC Sender
```

对于 KBEngine，Bridge 在 Python 脚本环境接收外部命令后，应将操作排入引擎允许的 callback 调度上下文。具体线程安全保证需要针对目标 KBEngine 版本验证。

---
title: 执行后端模型
---

# 执行后端模型

## 4.1 Backend 接口

所有执行后端实现统一的概念接口：

```text
ExecutionBackend
  ├── handshake()
  ├── initialize(config)
  ├── apply_plan(shard_plan)
  ├── start()
  ├── update_load(update)
  ├── stop_admission()
  ├── drain()
  ├── shutdown()
  ├── get_status()
  ├── get_metrics()
  └── subscribe_events()
```

这是一组领域操作，不要求所有后端实现为同一种 RPC 方法。协议必须显式描述不支持的能力。

### 4.1.1 Backend 状态

```text
Created
  -> Handshaking
  -> Ready
  -> Initialized
  -> PlanAccepted
  -> Running
  -> AdmissionStopped
  -> Draining
  -> Stopped

任意运行态 -> Degraded
任意运行态 -> Failed
Failed -> Recovering -> Handshaking
```

每次状态变化需要包含：

- backend_id
- swell_id
- state
- observed_at
- reason_code
- detail
- generation / epoch

不得仅靠 Controller 的命令发送成功推断 Backend 已完成状态转换。

## 4.2 Native Runtime Backend

由 Rust Worker 实现：

- Actor 生命周期。
- Session / Connection。
- Transport。
- Framing / Codec。
- Behavior Runtime。
- Virtual Clock。
- Timer Scheduler。
- Arrival Scheduler。
- 本地指标采集。
- 事件批量上报。

Native Runtime 的 Actor API 不应暴露给外部引擎 Bridge。

## 4.3 Engine Bot Backend

复用游戏引擎已有 Bot：

- Supervisor 管理 Bot 进程或容器。
- Bridge 运行在引擎提供的扩展环境。
- 引擎自己管理 Bot 的连接、协议和实体对象。
- Bridge 把引擎可见的控制能力映射为 Backend API。
- 对不能实现的操作返回明确的 Unsupported / CapabilityUnavailable。
- 不得伪造单 Bot 控制能力。

## 4.4 混合执行

一个 Swell 可以包含多个 Backend：

```yaml
execution:
  backends:
    - id: native-east
      type: native
      weight: 0.7
    - id: kbengine-native
      type: engine_bot
      adapter: kbengine
      weight: 0.3
```

上述 `weight` 仅为分配示意。实际计划必须展开成确定的 Backend 分配和 ShardPlan，不能在 Worker 端随机分配。

混合执行必须：

- 在报告中分开呈现 Backend 结果。
- 记录每种 Backend 的能力和调度精度。
- 对同一业务场景进行语义对齐测试。
- 不把不同客户端实现的结果默认视为严格等价。
- 提供按 Backend 对比的 CPU、内存、连接数和有效请求率。

---
title: Native Runtime 设计
---

# Native Runtime 设计

## 9.1 核心模块

```text
runtime/
  actor/
  clock/
  scheduler/
  arrival/
  session/
  transport/
  protocol/
  behavior/
  metrics/
```

首版避免拆分过多 Crate。逻辑模块可以先放在少量 Crate 中。

## 9.2 Actor 生命周期

```text
Planned
 -> Scheduled
 -> Creating
 -> Connecting
 -> Authenticating
 -> Active
 -> Draining
 -> Closed

任意阶段 -> Failed
```

每个状态转换必须记录时间戳和原因。

## 9.3 Virtual Clock

至少支持：

- Manual Clock：测试和确定性回放。
- Real Clock：生产执行。
- Timer Queue：定时事件。
- Think Time：行为等待分布。
- Arrival Scheduler：Bot 到达调度。
- Seeded RNG：可复现随机源。

时钟抽象不得让业务行为直接读取系统墙上时钟。需要系统时间时通过明确的 Clock API 获取，并标记其非确定性。

## 9.4 Scheduler

需要测量：

- timer insert / cancel / expire。
- 到期队列延迟。
- Actor runnable queue 长度。
- planned_at 与 actual_start 的偏差。
- 单次 tick 执行时间。
- 事件循环 lag。

必须做 5k / 20k / 50k Actor，每 Actor 4 个 timer 的基准测试。Tokio 时间轮的实现细节不能替代端到端运行时基准。

## 9.5 Transport / Protocol

分层：

- Transport：TCP / UDP / WebSocket 等实际需要的连接类型。
- Framing：消息边界。
- Schema：消息定义和版本。
- Codec：编码与解码。
- Session Adapter：登录、心跳、重连和会话状态。
- Behavior：业务动作。

Schema 不等于 Codec。Transport 不应知道游戏业务消息含义。

## 9.6 Behavior

v1 Native Runtime 可先使用 Rust 内部 Behavior trait。

行为上下文仅暴露：

- 注入时钟。
- 注入随机源。
- 发送动作。
- 订阅消息。
- 定时器。
- Actor state。
- 资源预算。
- 结构化日志。

不暴露 Worker 内部对象、Socket、线程句柄和全局 Runtime。

Lua 暂不进入 v1。未来如引入，必须实现指令预算、沙箱、确定性 RNG、时间注入和隔离测试。

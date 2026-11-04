---
title: KBEngine Adapter 实施设计
---

# KBEngine Adapter 实施设计

## 6.1 目标

第一款 Engine Bot Backend 选择 KBEngine，原因是：

- 已有原生 Bots。
- C++ 网络运行时与 Python 行为脚本边界明确。
- 可以优先复用已有 Bot 连接和协议实现。
- 有 Python 脚本扩展入口，适合实现 Bridge。
- 不需要把 C++ client_lib 链接进 Rust Worker。

## 6.2 目标部署形态

```text
Host
├── wavemaker-controller
└── wavemaker-supervisor
    ├── KBEngine Bots Process A
    │   ├── KBEngine C++ Runtime
    │   └── Python Bridge
    └── KBEngine Bots Process B
        ├── KBEngine C++ Runtime
        └── Python Bridge
```

第一版不要求 Docker。Supervisor 通过本机进程启动器启动 Bot 二进制。后续可以增加 ContainerProcessLauncher。

## 6.3 Python Bridge 模块

建议逻辑文件：

```text
engine-adapters/kbengine/
├── bridge/
│   ├── __init__.py
│   ├── bootstrap.py
│   ├── client.py
│   ├── command_queue.py
│   ├── event_collector.py
│   ├── metrics_collector.py
│   ├── bot_registry.py
│   ├── capability.py
│   └── protocol/
├── supervisor/
│   ├── process_spec.rs
│   ├── launcher.rs
│   ├── health.rs
│   └── adapter.rs
├── tests/
│   ├── protocol_conformance/
│   ├── integration/
│   └── failure/
└── README.md
```

该目录是逻辑建议，不要求 Python 代码和 Rust 代码处于同一语言构建系统。

## 6.4 KBEngine Bridge 生命周期

1. KBEngine Bots 启动。
2. 引擎加载 Bridge bootstrap。
3. Bridge 读取 Supervisor 注入的 IPC 地址、token、Swell ID 和 Backend ID。
4. Bridge 建立 IPC 连接。
5. 发送 Handshake。
6. Supervisor 校验版本和能力。
7. Controller 下发 Initialize。
8. Supervisor 转发计划和启动命令。
9. Bridge 将命令排入引擎 callback。
10. Bridge 收集 Bot 状态、事件和指标。
11. Swell 结束时先 StopAdmission。
12. 根据 DrainPolicy 清理或等待已有 Bot。
13. Bridge 发送最终状态和指标。
14. 断开 IPC，进程按策略退出。

## 6.5 KBEngine API 验证清单

下列接口是 ADR/已有讨论中提及的接口线索，不能仅凭名称推断所有语义。必须针对实际使用版本检查源码和运行行为：

| API / 机制 | 要验证的问题 |
|---|---|
| `KBEngine.addBots()` | 参数含义、创建节奏、返回值、失败回调、最大批次 |
| `KBEngine.bots` | 容器类型、更新时机、Bot 标识稳定性 |
| `KBEngine.callback()` | 回调粒度、线程上下文、最大滞后、异常行为 |
| `KBEngine.cancelCallback()` | 取消竞态和回调句柄生命周期 |
| `onInit()` | Bridge 初始化时机和重复调用行为 |
| `onFinish()` | 退出时是否保证执行、是否可等待异步收尾 |
| Bot 脚本回调 | 登录成功、失败、断线、重连和实体状态事件 |
| 进程退出 | 是否有优雅退出 API，退出码是否可靠 |
| 配置注入 | 如何传递 Swell、Backend、IPC 和认证参数 |

### 6.5.1 禁止未经验证的能力声明

例如：

- `destroy_individual` 未验证前返回 `Unknown`。
- `exact_arrival` 如果只能按 callback 批次创建，应声明 `Unsupported` 或以实际精度声明为 `Degraded`。
- 如果只有总 Bot 数，没有登录成功数，则不能把总数当在线人数。
- 如果无法获得每 Bot 延迟，不得从进程级延迟推导每 Bot 延迟。

## 6.6 Mode A / Mode B / Mode C 的关系

原 ADR 的接入模式应保留，但需要统一到 Backend 模型下。

- Mode A：引擎托管 Bot。Wavemaker 通过 Bridge 控制和观测原生 Bot。
- Mode B：Wavemaker 管理执行器，但复用引擎提供的部分客户端能力。具体边界按现有架构定义，不得假设拥有 socket。
- Mode C：Wavemaker Native Runtime 自己持有连接和协议执行。

三种模式不是同一套 Actor 的不同编译选项，而是不同的执行后端/接入能力组合。最终映射必须以原始架构文档中 Mode A/B/C 的定义为准；如果后续重新定义，需单独记录 ADR。

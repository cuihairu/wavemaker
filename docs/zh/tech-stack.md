---
title: 技术栈决策
---

# 技术栈决策

## 3.1 语言选择

| 领域 | 技术 | 结论 |
|---|---|---|
| Controller / Web API | Rust + Axum + Tokio | 主选 |
| Native Worker | Rust + Tokio | 主选 |
| Supervisor | Rust + Tokio | 主选 |
| 核心协议 | Protobuf | 统一契约 |
| KBEngine Bridge | Python | 引擎脚本环境要求 |
| BigWorld Bridge | 按实际 SDK / Bot 环境决定 | 待验证 |
| Python SDK | Python | 后续提供 |
| Lua SDK | Lua | v1 暂不实现 |
| C++ Adapter | 独立进程或隔离 shim | 按需 |
| Go SDK | Go | 后续提供，不进入核心 |
| 报告分析 | Rust 主流程；Python 可作为离线分析工具 | 不影响核心 |

## 3.2 Rust 生态建议

| 用途 | 建议组件 | 备注 |
|---|---|---|
| Async Runtime | Tokio | 核心异步运行时 |
| HTTP | Axum | Controller API |
| RPC | Tonic | gRPC 控制通道 |
| Serialization | prost / serde | Protobuf 与配置 |
| Database | SQLx | 常驻 Controller 的元数据存储 |
| CLI | Clap | CLI |
| Logging / Tracing | tracing | 结构化日志 |
| Metrics | OpenTelemetry Rust + 自定义聚合 | 指标导出 |
| Histogram | HDR Histogram 兼容实现 | 统一参数与 blob 版本 |
| Parquet | parquet-rs / Arrow | 版本与性能需验证 |
| Report Query | DuckDB | 可作为外部分析引擎，不强制嵌入 |

依赖版本应在 Phase 0 建立版本矩阵和最小可运行原型后冻结。不要在计划书中提前写死未经验证的 crate 版本。

## 3.3 是否必须引入数据库

- CLI-only 单次 Swell：可以使用文件系统、Parquet、JSON 和 DuckDB，不需要常驻数据库。
- 常驻 Dashboard、多用户、多 Swell 并发管理：需要持久化 Controller 元数据，建议 PostgreSQL。
- 单机开发模式：可选 SQLite。
- 压测明细和高频时间序列不应全部写入 PostgreSQL；优先文件分区和 Parquet。

Controller 的元数据存储与实验数据存储必须分离。

---
title: 分阶段实施计划
---

# 分阶段实施计划

## Phase 0：语义闭合与技术验证

**目标：在正式扩展前消除架构级不确定性。**

任务：

- [ ] WM-0001：整理原始架构文档中的 Mode A/B/C 精确定义。
- [ ] WM-0002：确定 Swell、Spectrum、Basin、Backend、Worker、Actor 的领域模型。
- [ ] WM-0003：闭合 arrival / concurrency / lifetime 语义。
- [ ] WM-0004：定义 saturation policy。
- [ ] WM-0005：验证指数到达与不同 lifetime 分布下的并发曲线。
- [ ] WM-0006：制定 plan-as-data 和 ShardPlan 冻结规则。
- [ ] WM-0007：确定 HDR V2 blob 参数和跨实现 conformance test。
- [ ] WM-0008：检查目标 KBEngine 版本的 Bot API 和 callback 源码。
- [ ] WM-0009：验证 KBEngine Bridge 能否在引擎脚本上下文安全执行命令。
- [ ] WM-0010：测量 KBEngine callback 调度粒度和 Bridge 队列延迟。
- [ ] WM-0011：建立 Rust Worker 最小 timer benchmark。
- [ ] WM-0012：确定 v1 是否需要常驻 Dashboard 和多用户管理。
- [ ] WM-0013：冻结 Protobuf 包名、版本兼容策略和错误码约定。
- [ ] WM-0014：明确 Swell 数据持久化边界。

交付物：

- ADR-005：执行后端与原生 Bot 接入决策。
- ADR-006：Spectrum 到达与并发语义。
- Protobuf v1 初稿。
- KBEngine API 验证报告。
- Timer / Actor Runtime benchmark 报告。

退出条件：

- BLOCKER-1 关闭。
- KBEngine 至少确认 L0/L1 的可行能力边界。
- Rust Runtime 基准测试有可复现结果。
- v1 控制面部署形态明确。

## Phase 1：协议与最小控制面

**目标：先建立可独立运行的 Controller、Worker 和 Supervisor 通信骨架。**

任务：

- [ ] WM-0101：创建 Rust workspace。
- [ ] WM-0102：实现 wm-protocol Protobuf codegen。
- [ ] WM-0103：实现统一错误码和 OperationResult。
- [ ] WM-0104：实现 Controller Axum API 最小骨架。
- [ ] WM-0105：实现 Worker gRPC 注册和心跳。
- [ ] WM-0106：实现 Backend capability handshake。
- [ ] WM-0107：实现 Supervisor 进程管理抽象。
- [ ] WM-0108：实现 NativeProcessLauncher。
- [ ] WM-0109：实现 Backend 状态机。
- [ ] WM-0110：实现 Swell 元数据与本地文件存储。
- [ ] WM-0111：实现 CLI 的 validate / run / status / stop。
- [ ] WM-0112：实现最小结构化日志与 tracing。
- [ ] WM-0113：实现 Controller / Worker / Supervisor 集成测试。

交付物：

- 可启动的 Controller。
- 可注册的空 Worker。
- 可启动/停止测试进程的 Supervisor。
- 最小 CLI。
- Protobuf v1 契约。

退出条件：

- Controller 能注册、监控和停止一个 Worker。
- Supervisor 能启动、检测和停止一个子进程。
- 进程异常退出可被正确记录。
- 协议版本不兼容时能够拒绝握手。

## Phase 2：KBEngine Bridge MVP

**目标：证明原生 Bot 可被统一管理、观测和纳入 Swell。**

任务：

- [ ] WM-0201：创建 KBEngine Bridge Python bootstrap。
- [ ] WM-0202：实现 Bridge 本机 IPC 客户端。
- [ ] WM-0203：实现 Handshake 和 capability report。
- [ ] WM-0204：实现 Supervisor 启动 KBEngine Bots。
- [ ] WM-0205：实现 Swell / Backend / generation 注入。
- [ ] WM-0206：实现命令队列和引擎 callback 调度。
- [ ] WM-0207：实现 ApplyPlan 的批次计划映射。
- [ ] WM-0208：实现 Bot 数量采集。
- [ ] WM-0209：实现连接和登录事件采集（以目标版本可用 API 为准）。
- [ ] WM-0210：实现 StopAdmission。
- [ ] WM-0211：实现 Drain / Shutdown。
- [ ] WM-0212：实现 Bridge 心跳与断线重连。
- [ ] WM-0213：实现事件序列、cursor 和重复事件去重。
- [ ] WM-0214：实现进程 CPU / RSS / FD 采集。
- [ ] WM-0215：实现 Bridge 队列长度和 callback lag 指标。
- [ ] WM-0216：实现 KBEngine adapter conformance test。
- [ ] WM-0217：实现端到端登录压测样例。
- [ ] WM-0218：实现 Bridge / Supervisor / Bot 故障注入测试。

交付物：

- KBEngine Bridge。
- KBEngine Supervisor Adapter。
- KBEngine Backend capability profile。
- 一份包含 Bot 事件和 Generator 指标的 Swell 报告。

退出条件：

- 不修改 KBEngine C++ 网络运行时即可接入。
- 可以启动 Bot、观察登录状态、停止新增。
- Bridge 失联不会直接清空已有连接。
- 所有未知能力都显式标记。
- 连续运行测试无明显事件丢失或资源泄漏。

## Phase 3：Native Rust Runtime

**目标：实现 Wavemaker 自己的高性能 Actor 执行后端。**

任务：

- [ ] WM-0301：实现 Actor 状态机。
- [ ] WM-0302：实现 Clock trait 和 ManualClock。
- [ ] WM-0303：实现 RealClock。
- [ ] WM-0304：实现 Timer Scheduler。
- [ ] WM-0305：实现 Seeded RNG。
- [ ] WM-0306：实现 Arrival Scheduler。
- [ ] WM-0307：实现 Actor 生命周期管理。
- [ ] WM-0308：实现 Session 状态机。
- [ ] WM-0309：实现 Transport trait。
- [ ] WM-0310：实现首个 TCP Transport。
- [ ] WM-0311：实现 Framing。
- [ ] WM-0312：实现 Schema / Codec 分离。
- [ ] WM-0313：实现 Behavior trait。
- [ ] WM-0314：实现消息事件分发。
- [ ] WM-0315：实现批量 Actor 调度。
- [ ] WM-0316：实现 Worker 本地 Metrics。
- [ ] WM-0317：实现 HDR histogram 和 V2 blob。
- [ ] WM-0318：实现 coordinated omission 修正。
- [ ] WM-0319：实现 5k / 20k / 50k benchmark。
- [ ] WM-0320：实现 24h soak test。
- [ ] WM-0321：实现 Native Backend conformance test。

交付物：

- Rust Native Worker。
- Actor Runtime。
- TCP 协议适配示例。
- 性能基准报告。

退出条件：

- 同一 plan + seed 可重现计划。
- Actor 状态机和时钟测试通过。
- 目标规模下发生器自身不成为明显瓶颈。
- planned_at 与 actual_at 偏差满足项目门槛。
- 24h soak 通过。

## Phase 4：Basin 与统一观测

**目标：将 Generator、Server 和计划负载统一到 Swell 时间线。**

任务：

- [ ] WM-0401：实现 Basin 数据模型。
- [ ] WM-0402：实现服务拓扑模型。
- [ ] WM-0403：实现 Metric Catalog。
- [ ] WM-0404：实现 Prometheus Provider。
- [ ] WM-0405：实现 Metric Mapping。
- [ ] WM-0406：实现 Query Scheduler。
- [ ] WM-0407：实现 source_timestamp / observed_at / collected_at。
- [ ] WM-0408：实现数据缺失和延迟检测。
- [ ] WM-0409：实现 Preflight。
- [ ] WM-0410：实现 observer_baseline。
- [ ] WM-0411：实现观测开销评估和降级。
- [ ] WM-0412：实现 Generator Metrics 聚合。
- [ ] WM-0413：实现 Server Metrics 聚合。
- [ ] WM-0414：实现时间线对齐。
- [ ] WM-0415：实现报告中的数据质量标签。
- [ ] WM-0416：实现端到端关联报告。

交付物：

- Basin 配置。
- Prometheus Provider。
- Generator / Server 统一时间线。
- 含数据质量的报告。

退出条件：

- 可以证明目标服务拓扑覆盖情况。
- 缺失数据不会被当成零。
- 观测开销可测量。
- Server Observability 不可用时报告正确降级。

## Phase 5：分布式执行与恢复

**目标：从单机执行扩展到多 Worker、多 Backend。**

任务：

- [ ] WM-0501：实现 Worker Registry。
- [ ] WM-0502：实现 Backend Registry。
- [ ] WM-0503：实现资源容量描述。
- [ ] WM-0504：实现 ShardPlan。
- [ ] WM-0505：实现 Swell 创建时冻结 Worker 数量。
- [ ] WM-0506：实现租约和心跳。
- [ ] WM-0507：实现失联冻结新增。
- [ ] WM-0508：实现 Backend generation。
- [ ] WM-0509：实现恢复握手和状态同步。
- [ ] WM-0510：实现多 Worker Metrics 聚合。
- [ ] WM-0511：实现跨 Worker HDR 合并。
- [ ] WM-0512：实现 Controller 重启恢复。
- [ ] WM-0513：实现 Worker 故障注入。
- [ ] WM-0514：实现多 Backend 混合 Swell。
- [ ] WM-0515：实现分布式 24h soak。

交付物：

- 多 Worker Swell。
- 故障恢复机制。
- 混合 Native + KBEngine 执行报告。

退出条件：

- 分片计划不发生静默漂移。
- 租约失效不产生额外断连尖峰。
- 旧 generation 事件不会污染新状态。
- 跨 Worker 指标合并通过 conformance test。

## Phase 6：Dashboard 与扩展生态

**仅在 v1 产品形态需要时启动。**

任务：

- [ ] WM-0601：实现 Swell 列表与详情。
- [ ] WM-0602：实现 Backend / Worker 状态页面。
- [ ] WM-0603：实现 Spectrum 编辑和校验展示。
- [ ] WM-0604：实现负载时间线。
- [ ] WM-0605：实现 Generator / Server 指标曲线。
- [ ] WM-0606：实现报告对比。
- [ ] WM-0607：实现用户和权限。
- [ ] WM-0608：实现审计日志。
- [ ] WM-0609：评估 Docker / Podman Launcher。
- [ ] WM-0610：评估 Lua Behavior SDK。
- [ ] WM-0611：评估 Go / C++ / Python 第三方 SDK 完整性。

---

---

# v1 范围与明确不做项

## 19.1 v1 必须有

- Rust Controller。
- Rust Native Worker。
- Rust Supervisor。
- Protobuf 契约。
- Spectrum / Swell / ShardPlan。
- KBEngine Bridge MVP。
- Generator Observability。
- Basin 基础模型。
- Prometheus Provider。
- Parquet / JSON 报告数据。
- CLI。
- conformance / integration / soak tests。

## 19.2 v1 可选

- Dashboard。
- PostgreSQL。
- Docker / Podman。
- Python 通用行为插件 SDK。
- BigWorld Adapter（取决于 SDK 和测试环境）。

## 19.3 v1 不做

- Kubernetes 编排。
- 插件市场。
- Lua 沙箱。
- 通用动态库 ABI 插件。
- 每个 Bot 一个容器。
- 全量日志和 Trace 存储平台。
- 自动根因诊断承诺。
- 无限制的任意语言嵌入式运行时。

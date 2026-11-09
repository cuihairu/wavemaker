# Wavemaker Todo

Task checklist from the phased implementation plan (Phase 0–6). Keep WM task IDs stable.

## Phase 0: Semantic Closure and Technical Verification

- [x] WM-0001: Pin down the precise definitions of Mode A/B/C in the original architecture document. *(2026-11-09 冻结于 docs/architecture.md「Mode A/B/C Definitions」+ docs/zh/architecture.md 1.4 节镜像：以「谁建 Bot / 谁持 socket / 控制观测粒度」三问钉死三模式；Mode A 能力主张引用 docs/spec/kbengine-api-audit.md 的 file:line 证据（addBots 批次累加无单控 bots.cpp:344-388；KBEngine.bots 只读映射 pybots.cpp:16-113）；原始架构文档不在本地，定义以计划书 §6.6 + 已审计源码事实为准，若原始文档重现以新 ADR 调和；ADR-005 D2 的 Mode B「待确认」行已更新为已冻结、v1 不实现、实现前需单独 ADR)*
- [x] WM-0002: Define the domain model for Swell, Spectrum, Basin, Backend, Worker, Actor. *(docs/glossary.md)*
- [x] WM-0003: Close the arrival / concurrency / lifetime semantics. *(docs/spec/spectrum-semantics.md)*
- [x] WM-0004: Define saturation policies. *(docs/spec/spectrum-semantics.md §5)*
- [x] WM-0005: Verify concurrency curves for exponential arrival under different lifetime distributions. *(tools/spectrum_sim.py + tools/test_spectrum_sim.py; results in docs/spec/spectrum-semantics.md §7)*
- [x] WM-0006: Define plan-as-data and ShardPlan freezing rules. *(docs/adr/ADR-006-plan-as-data.md)*
- [x] WM-0007: Fix HDR V2 blob parameters and cross-implementation conformance tests. *(2026-11-09 冻结 v1 参数：2 位有效数字、ns 单位、1..=3.6e12（1 小时）→ crates/wm-core/src/hdr.rs（V2 常量 + 几何/索引整数运算 + zigzag LEB128-64b9B 编解码 + zlib 压缩 blob encode/decode + Java 口径 min/max/mean/percentile）；对拍四实现：HdrHistogram Java 2.2.2 + hdrhistogram-go v1.4.0 真库生成器（tools/hdr_conformance/，反射读真库内部）→ compare.py 独立 Python 第三实现三方校验 → fixtures/hdr_conformance.json 入库（10 参数矩阵行、frozen 全量 4608 槽索引映射 + 25 探针、small/frozen-v1/empty 三个值集的 Java+Go blob）；hdr.rs 9 unit tests + hdr_conformance.rs 7 tests 对拍（几何、索引映射、统计、Java/Go blob 解码 counts 逐一相等、重编码 roundtrip）；已知分歧记录不隐藏：Go percentile 截断 vs Java ceil（Rust 从 Java），fixture meta 带出处与日期；cargo test --workspace 107 绿)*
- [x] WM-0008: Inspect the target KBEngine version's bot API and callback source code. *(static audit closed: docs/spec/kbengine-api-audit.md — file:line evidence at fork commit bf1a9cd6; 7 plan-vs-source discrepancies logged; runtime items deferred to WM-0009/WM-0010 checklist)*
- [x] WM-0009: Verify that the KBEngine Bridge can execute commands safely in the engine scripting context. *(2026-11-09 静态闭合：§8 清单 8/8 项静态验证完成（S1–S8，docs/spec/kbengine-api-audit.md §8.1，基线 fork 3082ab50）——单线程、阻塞影响面、异常吞噬、telnet 无 GIL 竞争、ScriptID 复用、onInit 单次、bots 映射快照、addBots 节拍（推翻 bots.h 注释）全部有 file:line 证据；Phase 2 实现约束已导出（§8.3）。2026-11-10 收口：Bridge 已落地（WM-0201/0202）并按 §8.3 约束实现、复测通过——动态证据（engine-adapters/kbengine/tests，30 tests 绿×8 轮稳定，真实 AF_UNIX/TCP loopback 套接字）：#1 IPC 接收线程只入队绝不调脚本 API（test_receiver_thread_only_enqueues_never_touches_engine 锁定 recv 循环无脚本调用）+ test_concurrent_producers_execute_exactly_once_on_pump_thread（8 生产者×50 命令并发入队，泵按引擎方式单发触发逐 tick 消费，全部命令 exactly-once、engine.scheduled 恒 1、零 cancel）；#3 只 cancel 活句柄（test_on_finish_is_synchronous_and_cancels_pump）；#5 onFinish 同步收尾、泵对处理器异常免疫照常重臂（test_callback_exception_does_not_stop_pump，S3「打印即吞」语义）；#4 exact_arrival=degraded+batch/quantum 界（bridge/capability.py，S8）；端到端命令路径（Supervisor→帧→接收线程→CommandQueue→泵执行）经 test_bootstrap_wires_ipc_client_lifecycle 全程走通。残留非本项问题域：RT2（嵌入解释器 PyErr_PrintEx×SystemExit/excepthook 路径）与 RT1/RT4 量化实测归 WM-0010/WM-0216，BLOCKED 于无部署集群，不虚标）*
- [ ] WM-0010: Measure KBEngine callback scheduling granularity and Bridge queue latency. *(静态部分已闭合：粒度=1/gameUpdateHertz 默认 100ms 量化下界（audit §3）；addBots 到达精度上界≈1s 批+gameTick（§8.1 S8）。实测（RT1/RT4）BLOCKED：需运行中的 KBEngine 集群+bots 进程，本机 kbengine-build 仅存构建产物无部署、无 Bridge 被测对象；不虚标数字。2026-11-09 巡检：BLOCKED，标注跳过，待 Bridge 集群就绪后实测)*
- [x] WM-0011: Build a minimal Rust Worker timer benchmark. *(crates/wm-core/benches/timer_scheduler.rs (criterion); baseline results in docs/benchmarks/timer-benchmark.md — load-sensitive, relative comparisons only)*
- [x] WM-0012: Decide whether v1 needs a resident Dashboard and multi-user management. *(ADR-008（docs/adr/ADR-008-v1-dashboard-scope.md，Accepted 2026-11-09）：v1 不做常驻 Dashboard、不做多用户——交付面=nightly CLI+报告工件+CLI 所需最小 REST/gRPC；单操作员信任模型；Phase 6 不随 v1 启动，前置=WM-0600 形态决策；报告含 qps/tps 曲线（WM-0417 口径）即 v1 可视化替代)*
- [x] WM-0013: Freeze Protobuf package names, version compatibility policy, and error code conventions. *(docs/spec/protobuf-conventions.md; error-code skeleton, field freeze in Phase 1 WM-0103)*
- [x] WM-0014: Clarify the persistence boundary for Swell data. *(docs/spec/swell-persistence.md（2026-11-09 冻结）：四类数据责任表——metadata/timeline 归 Wavemaker（SwellStore：原子重写+追加 JSONL+路径段校验）、evidence 归 Provider 写（目录约定预创建）、Basin 时序归外部 Provider（报告只引用）、派生报告工件不计入边界；明确不落盘项：逐请求明细（HDR 为持久化粒度）、Worker 运行时指标、计划文件（仅存 spectrum_path 引用）；与 wm-core::store 11 tests 逐条对齐)*

## Phase 1: Protocol and Minimal Control Plane

<!-- 对账 2026-11-06：以下勾选项均基于已落 commit + 绿色 cargo test（本地 80 tests）验证；未勾项为真实缺口，不虚标。 -->

- [x] WM-0101: Create the Rust workspace. *(1e2aced；cargo workspace 7 crates；cargo test/clippy/fmt 全绿)*
- [x] WM-0102: Implement wm-protocol Protobuf codegen. *(f8823fe；prost/tonic codegen，5 个 v1 包；wm-protocol 5 unit tests)*
- [x] WM-0103: Implement unified error codes and OperationResult. *(f8823fe + b1b419d；proto/common/v1/common_types.proto ErrorCode/OperationResult + docs/spec/protobuf-conventions.md；5 unit tests)*
- [x] WM-0104: Implement a minimal Controller Axum API skeleton. *(9ed9a013；axum /healthz + /api/v1/status + gRPC ControlService；http.rs 3 tests + controller state 14 tests)*
- [x] WM-0105: Implement Worker gRPC registration and heartbeat. *(703a7b5d；WorkerSession::connect + heartbeat loop；worker 2 tests 含协议版本拒绝路径)*
- [x] WM-0106: Implement Backend capability handshake. *(476a2e1；wm-worker NativeBackendService 落 backend/v1 契约：Handshake/ApplyPlan/Start/Stop，协议版本不符在握手即拒且不推进状态；backend_stack.rs 4 个真实 gRPC 栈测试)*
- [x] WM-0107: Implement the Supervisor process management abstraction. *(0638833；ProcessSupervisor trait start/terminate/kill/status/usage，wm-supervisor)*
- [x] WM-0108: Implement NativeProcessLauncher. *(0638833；含 Capture/Inherit 流与 ExitRecord；8 unit tests)*
- [x] WM-0109: Implement the Backend state machine. *(95fb3fa + 49c1b51 + 476a2e1；wm-core BackendStateMachine 线性生命周期 + Degraded/Failed/Recovering + generation 守卫；core 24 tests，worker 侧经 BackendService 集成)*
- [x] WM-0110: Implement Swell metadata and local file storage. *(wm-core::store——SwellStore 本地文件存储：`<root>/<swell_id>/` 下 metadata.json（temp+rename 原子写）与 timeline.jsonl 事件时间线分离，evidence/ 数据子树按 `<swell_dir>/evidence/<assertion-name>/<timestamp>/` 约定预创建（ADR-007/assertion-provider §6），swell id/断言名路径段校验防逃逸；11 unit tests；遵循 tech-stack.md 文件系统+JSON、元数据与实验数据分离）*
- [x] WM-0111: Implement CLI validate / run / status / stop. *(validate 原已真实现；run/status/stop 接控制器 REST：POST/GET /api/v1/runs + /runs/{id}/stop + GET /api/v1/status，wm-cli 8 unit tests + cli_stack.rs 3 个 CLI↔控制器栈测试；无效 spectrum 在联系控制器前即以退出码 1 拒绝)*
- [x] WM-0112: Implement minimal structured logging and tracing. *(wm-observability::init——JSON 默认/WM_LOG_FORMAT=text/RUST_LOG 级别，幂等 init；2 unit tests；wm-cli 已接入)*
- [x] WM-0113: Implement Controller / Worker / Supervisor integration tests. *(crates/wm-controller/tests/full_chain.rs：真实 gRPC+HTTP 控制器 + WorkerSession 注册/心跳 + HTTP status 可见 + NativeProcessLauncher start/status/terminate + 异常退出记录 + 协议版本握手拒绝；4 个全链路测试)*

## Phase 1.5: Daily build and distribution（每日构建）

<!-- 用户反复要求：每日构建必须落在 Releases 页的滚动 nightly，不能只躺在 Actions artifacts 里。
     现状：nightly.yml 已改成 schedule 即发布到滚动 nightly Release（48h 门禁 + 两个架构 + SHA256 + draft=false + 落地断言）。 -->

- [x] WM-DB-1: Replace the artifact-only nightly with a rolling `nightly` Release workflow. *(.github/workflows/nightly.yml — schedule 即 publish，不再「releases 是独立动作」)*
- [x] WM-DB-2: 48h staleness gate; `workflow_dispatch` always builds. *(gate job，schedule 才看 HEAD 年龄)*
- [x] WM-DB-3: Build `wm` release for linux-x86_64 + linux-aarch64. *(build matrix)*
- [x] WM-DB-4: Smoke gate: `wm --version`, good config exit 0, bad config non-zero. *(Smoke gate step)*
- [x] WM-DB-5: Per-platform asset `wavemaker-<platform>` + `.sha256` + platform notes. *(Stage step)*
- [x] WM-DB-6: Publish to the single rolling `nightly` Release (delete-and-recreate), notes carry commit + asset list. *(publish job)*
- [x] WM-DB-7: Publish runs on `schedule`, not dispatch-only. *(publish 无 event 条件)*
- [x] WM-DB-8: `gh release edit nightly --draft=false` + assert non-empty publishedAt and assets. *(publish job 尾部断言)*
- [x] WM-DB-9: Dispatch by hand once and verify the `nightly` Release shows fresh assets; confirm no version tag appeared. *(run 42494394105 全绿；nightly Release 6 assets、draft=false、publishedAt 2026-11-08T15:33:01Z；下载 x86_64 二进制实测 --version=wm 0.1.0、sha256 匹配、validate 通过；无版本 tag，仅 refs/tags/nightly)*
- [x] WM-DB-10: Add a README section pointing at the Releases page with an asset→platform table. *(README.md + README.zh.md「每日构建」节)*

## Phase 2: KBEngine Bridge MVP

- [x] WM-0201: Create the KBEngine Bridge Python bootstrap. *(engine-adapters/kbengine/bridge/bootstrap.py——Bootstrap 状态机 standby→running→stopped（on_init 二次调用守卫 §8.1 S6）、单次 100ms 泵（KBEngine.callback 恰 2 参，经 engine_port.py:32 端口；泵异常打印吞掉后照常重臂 §8.1 S3）、BootstrapConfig 四环境变量注入（WAVEMAKER_BRIDGE_IPC/TOKEN/SWELL_ID/BACKEND_ID）；capability.py 诚实能力目录（exact_arrival=degraded + arrival_batch_ms=1000/resolution_ms=100，§8.1 S8）；engine-adapters/kbengine/kbemain.py 入口脚本 onInit(0)/onFinish 委托；tests/test_bootstrap.py 10 unit tests 经 FakeEngine 端口，无引擎运行时依赖)*
- [x] WM-0202: Implement the Bridge local IPC client. *(engine-adapters/kbengine/bridge/ipc_client.py——parse_address:68 支持 unix:/裸路径/tcp:host:port（bridge.md 双边界）、线格式 `[u32 BE len][UTF-8 JSON]` 与 crates/wm-runtime/src/framing.rs 同前缀、字段名镜像 proto/engine/v1/engine_bridge.proto（嵌入解释器无 protobuf，不引入依赖）；FrameReader:111 半帧缓冲+多帧合并+MAX_FRAME_BYTES 拒绝越界/非 JSON 对象（越界即断连，长度前缀流不可重同步）；IpcClient:211 收发双线程：接收线程只入队 CommandQueue 绝不调脚本 API（§8.3 #1/S1，test_receiver_thread_only_enqueues 锁定），send_message:322 只入队不落 socket（S2：内核缓冲满不得拖死主循环），序列化在发送线程做（引擎线程不付 JSON 开销）；connect:251 有界连接（2s）失败仅计数不抛（audit §5：onInit 抛异常会杀死 bots 进程）；close:286 同步有界 join、排空残余计入 dropped（§8.3 #5 无异步收尾）；OutboundQueue:148 有界丢弃最新+put_back:190 保证停机路径上每条消息「已发送/计 dropped/计 encode_error」三者恰居其一（测试抓到发送线程停机时静默吞消息的真实缺陷）；bootstrap.py:142/161 on_init 启动、on_finish 内联关闭、send_message:169/ipc_stats:177 转发；心跳/重连归 WM-0212，本模块只报状态与计数；tests/test_ipc_client.py 19 tests 全部经真实 loopback 套接字（AF_UNIX+TCP 双栈）)*
- [x] WM-0203: Implement Handshake and capability report. *(engine-adapters/kbengine/bridge/handshake.py——build_handshake:60 镜像 wavemaker.engine.v1.EngineHandshake 字段（protocol_version/engine/adapter/backend_id）+ 完整诚实能力目录（拒绝静默降级）；parse_handshake_ack:47 仅 code=OK 接受、generation 非整数钳 0、非 ack 形状返回 None 不抛入脚本线程（S3）、malformed result 给稳定本地 reason=malformed_ack；bootstrap.py:on_init 连接后自动发首帧握手（send 仅入队，on_init 保持轻量不拖启动）；ack OK→ready+handshake_generation；拒绝→refused：内联 close 有界 join（握手期无 bots，停顿无害）、泵取消、状态机熄火但引擎继续运行不崩 bots（audit §5，docs/bridge.md 拒绝语义）；泵在 running/ready 重臂、refused 终止；tests/test_ipc_client.py +7：握手载荷镜像 proto+capability_report、accept→ready（gen 42）、UNSUPPORTED 拒绝→refused+会话拆除+无流量、未知 kind 记录不动状态机、parse/build 单测（malformed/非 OK/garbage）；35 tests 绿 ×12 轮稳定（真实 loopback）；另修 LoopbackServer.close 与 accept 线程竞态（join 后再关，peer-disconnect 测试不再等永不来的 FIN）与 3 处 sent 计数可见性轮询)*
- [x] WM-0204: Implement Supervisor startup of KBEngine Bots. *(crates/wm-supervisor/src/kbengine.rs——BotsProcessSpec: bots 可执行+参数+工作目录+四个 Bridge 注入变量（BRIDGE_IPC/TOKEN/SWELL_ID/BACKEND_ID_ENV 常量，测试锁定与 engine-adapters/kbengine/bridge/bootstrap.py BootstrapConfig.ENV_* 逐字一致——跨语言契约单一事实源）+extra_environment 合并；bridge_environment():24 输出精确注入集；to_process_spec():63 泛化为 ProcessSpec（双流 Capture 保失败日志尾、RestartPolicy::Never——S6 重启=新进程新 onInit，不自动重启除非 swell 失败策略明确允许，plan §7.3）；真实子进程测试：startup_injects_bridge_env_into_the_child 经 /bin/sh 回读验证注入变量到达子进程（unix:/tmp/wm-e2e.sock|sw-42|be-7）、early_death_is_recorded_with_stderr_tail 验证握手前早夭进程记录 exit 3+stderr 尾（plan §7.3 证据保留）；模块文档明确不做范围：Supervisor 侧 IPC listener 随 Backend service 工作、不自动重启；wm-supervisor 13 tests 绿 ×6 轮稳定，workspace 24 套件绿）*
- [x] WM-0205: Implement Swell / Backend / generation injection. *(crates/wm-supervisor/src/kbengine.rs——新增 BACKEND_GENERATION_ENV="WAVEMAKER_BACKEND_GENERATION" 常量（文档明确：launch hint，握手 ack 的 generation 才是权威，Supervisor 侧可据此对照自身记录）+ BotsProcessSpec.backend_generation: u64（0=unset）经 bridge_environment() 注入，契约测试锁定与 Python 侧逐字一致；engine-adapters/kbengine/bridge/bootstrap.py——BootstrapConfig.ENV_BACKEND_GENERATION + _parse_generation 防御解析（非正/非数字→0，onInit 绝不因注入坏值抛异常杀死 bots，audit §5）+ capability_report() 增加 backend_generation；handshake.py build_handshake 载荷带 backend_generation（文档注明：超出冻结 proto EngineHandshake 的加法扩展字段，ack generation 权威——Wire 上 Supervisor 可交叉核对）；tests：env 解析 valid/missing/malformed(负数/0/带空白/None/int 直传) → 0 或原值、capability_report 默认 0、握手单测载荷含 generation=12、端到端 loopback 测试 make_bootstrap(generation=6) 断言真实 wire 帧带 backend_generation=6 且 set(hs)==set(report)|{"kind"} 不变量保持；Python 36 tests 绿 ×3 轮、cargo fmt/clippy -D warnings/workspace 全绿（wm-supervisor 13 tests）)*
- [x] WM-0206: Implement the command queue and engine callback scheduling. *(bridge/commands.py——parse_command:64 镜像 proto EngineCommand 字段（operation_id/generation/command_name/payload，proto3 默认值、bool generation 拒绝、非命令形状返回 None 不抛入脚本线程 S3）；build_command_result:87 镜像 EngineCommandResult+wavemaker.common.v1.OperationResult（稳定字符串码 OK/INVALID_ARGUMENT/FAILED_PRECONDITION/UNSUPPORTED/INTERNAL，非 OK 必带 reason_code+detail，protobuf-conventions §3.2）；CommandDispatcher:109——握手前命令→FAILED_PRECONDITION not_ready、generation≠已分配→stale_generation（结果携带已分配 generation，§2.4 旧 generation 迟到流量不得生效）、未注册命令名→UNSUPPORTED unknown_command 绝不静默、handler 异常→INTERNAL handler_error（报告而非逃逸，泵 S3 兜底仍在外层）；bootstrap.py 接线：_handle_command 路由 ack→command→未知记录，set_command_handler 供 WM-0207+ 注册，结果经 _send_command_result 非阻塞入队 IPC 发送（S2）；命令队列（WM-0202 CommandQueue 只入队）+泵（WM-0201 单次 100ms 重臂）已有，本项补齐二者与 handler 间的分发层（docs/bridge.md §Command Queue 模型）；tests/test_ipc_client.py +6：parse/build 单测、握手前命令拒绝（wire 断言）、stale 拒+新鲜放行（同连接先拒后 OK）、未知名 UNSUPPORTED、handler 脚本线程执行+payload 直传+异常→INTERNAL+泵存活后续命令仍分发；幂等键（Start/Drain 按 operation_id、ApplyPlan 按 plan_digest）归 WM-0207+ handler，分发层只路由和封装；42 tests 绿 ×5 轮（真实 loopback）)*
- [x] WM-0207: Implement ApplyPlan batch plan mapping. *(bridge/plans.py——parse_plan_payload 校验 JSON ShardPlan 载荷（proto plan_payload 不透明 bytes 的 JSON 侧 v1 形状：swell_id/plan_digest/batches[{offset_ms,bot_count}]）并把 offset 量化到引擎 callback 量子（offset_ms//resolution）同桶合并求和（audit §3/D4 exact_arrival=degraded）；PlanState.apply 幂等按 plan_digest（同 digest 已接受→OK 重放，不同 digest→FAILED_PRECONDITION conflicting_plan 冻结分片绝不静默替换，spectrum.md §ShardPlan Freezing；swell 不匹配→INVALID_ARGUMENT swell_mismatch；running 中 apply→conflicting_plan）；PlanState.start 幂等按 operation_id（docs/bridge.md §Command Semantics；未接受 plan 即 start→FAILED_PRECONDITION plan_not_accepted；运行中换 operation_id→already_running）；due_batches 按 bucket 单调放行批次（D2 批次预算：每泵 tick 每到期批次至多一次 addBots——引擎侧 CreateAndLoginHandler 自身限速 S8，Bridge 不做紧循环）；bootstrap 接线：apply_plan/start handler 注册进 WM-0206 dispatcher（handler 签名扩展为 (payload, operation_id)，结构化 (code,reason,detail) 返回→结果信封而非 INTERNAL）、execute_due_batches 挂入泵 tick（S1 单线程，不额外武装引擎 callback，§8.3 #3）；engine_port.py 增加 add_bots→KBEngine.addBots(totalCount) 恰 1 参形式（audit §1：累加语义、无返回值无失败反馈 D1——失败只能由 WM-0208 清点差额暴露，FakeEngine 记录调用+累计）；tests/test_plans.py 11 unit（fake clock 驱动批次时序：bucket 0 随 start 即发、半桶不发、耗尽后恒空、同桶合并单次 addBots）+ test_ipc_client.py +3 wire（apply OK/重放 OK/冲突拒 digest 不变、畸形载荷 INVALID_ARGUMENT 稳定 reason 且 state 不变、start 全链：未接受拒→接受→start OK→bucket 0 经 FakeEngine.addBots 落账→同 operation_id 重放不重复 addBots→换 id 拒 already_running）；55 tests 绿 ×8 轮；StopAdmission/Drain/Shutdown 仍 UNSUPPORTED 归 WM-0210/0211)*
- [x] WM-0208: Implement bot count collection. *(bridge/bot_registry.py——collect_bot_snapshot 按 audit §2/§8.3 #2/S7 纪律：先取 keys() INCREF 快照（pybots.cpp:89-150 无重入构建、后续 delClient 不使其失效）再逐项守卫访问——KeyError（同 tick delClient 移除）与 isDestroyed（bots.cpp:286-290 守卫模式）均计入 dead_ids 绝不崩泵；存活数来自守卫后遍历而非裸 len()（len 读 live map 至多一个 tick 内计入已逻辑销毁的 client，差异窗口如实记录）；isDestroyed 属性存在于 Python 可见面的清单属待运行时验证项（audit §2 遗留）——getattr 默认 False 降级为「视为存活」，不虚构事实；bots 映射缺失→返回 None（不可清点≠0，绝不报假零，docs/kbengine.md no-unverified-claims）；键为 int32 appID（非 entityID 非序号）；engine_port.py 增加 EnginePort.bots 属性 + FakeBots 镜像 PyBots 契约（int appID 键、只读下标赋值 TypeError（mp_ass_subscript NULL）、keys() 快照、live KeyError、可挂 isDestroyed 客户端对象）；bootstrap.bot_snapshot() 泵线程访问器——addBots 失败对 Bridge 不可见（audit §1/D1 无返回值无回调），缺口以 bot_count 低于计划目标呈现给 Supervisor；wire 上报归事件/指标项（WM-0213/0215），本项交付清点原语；tests/test_bot_registry.py 8 unit（存活计数/守卫/快照后消失 KeyID 计 dead/无 isDestroyed 属性视为存活/None 不等于零/只读/Bootstrap 接线/无 bots 映射 None）；63 tests 绿 ×6 轮)*
- [x] WM-0209: Implement connection and login event collection (subject to APIs available in the target version). *(bridge/connection_events.py——ConnectionCollector 逐泵 tick diff KBEngine.bots keys() 快照（§8.3 #2/S7：keys() INCREF 快照、S1 单线程无撕裂、None 映射跳过保基线不崩）：新 clientID→BotCreated（CreateAndLoginHandler→bots.cpp:500-506 创建即登录尝试起点）、消失 clientID→BotDisconnected（delClient bots.cpp:286-290/515-525——踢/正常断/销毁在该层不可区分，一律如实报 BotDisconnected 不虚构细分）；bot_id=int32 appID（每 client 稳定，生命周期=client 在 bots 进程内存活期——满足 bridge.md 稳定 ID 要求）；**login 态明确不采**：BotConnected/BotLoginSucceeded/BotLoginFailed 需 C_STATE_*（clientobject.cpp:414-455）或 ClientObjectBase 事件流（EventData_onKicked clientobjectbase.cpp:240-247）暴露给 Python——audit §2 判定 Python 可见面属待运行时验证，能力目录保持 login_success/connected_count/per_bot_events=unknown（LOGIN_STATE_COLLECTABLE=False 与目录一致，无虚构；WM-0216 真机验证后旁挂接入）；首采为基线不追溯报量（避免启动洪泛）；有界缓冲 4096 丢最新计数（与 CommandQueue 同纪律）；裸记录 {event_type, bot_id, source_timestamp(monotonic)}，wire 信封（event_id/sequence/observed_timestamp/去重）归 WM-0213；bootstrap 接线：泵 tick 内 poll（S1）+ take_connection_events 排空；tests/test_connection_events.py 7 unit（基线不洪泛/created 带 appID/removed/disconnect、同 tick 增删有序（先 created 升序后 removed 升序）/None 保基线/溢出丢新留旧计数/泵自动采集）；70 tests 绿 ×6 轮)*
- [x] WM-0210: Implement StopAdmission. *(plans.py——PlanState.stop_admission：单向闸（docs/bridge.md 无 reopen 语义）——置 admission_stopped 后 due_batches 恒冻结（已排程未发批次不再发、绝不复活）、start 即使 plan_accepted 也→FAILED_PRECONDITION admission_stopped（admission 停后不得开新 bot 执行）、现有 bot 不动（命令语义=停新不拆旧连接）；幂等不带键（docs/bridge.md 表：StopAdmission 仅标 Idempotent——任意 operation_id 重复 stop 均 OK）；诚实边界如实记录：引擎侧 CreateAndLoginHandler 的「维持 N 存活」补建（audit §8.1 S8 注：bot 被踢后 delClient 缩量 handler 会补建）无静态已知 API 可取消——该缺口属 WM-0216 运行时验证项，不虚构已停创建；bootstrap 注册 stop_admission handler 进 dispatcher；tests/test_plans.py +3（幂等任意 id/运行中 stop 冻结余下批次+clock 前进 1s 恒空+换 op start 拒/idle stop 后 apply 仍可记 start 拒）+ test_ipc_client.py +1 wire（apply→start→bucket 0 落账→stop OK×2 幂等→换 op start 拒 admission_stopped→再泵 add_bots_calls 不再增长）；74 tests 绿 ×6 轮)*
- [ ] WM-0211: Implement Drain / Shutdown.
- [ ] WM-0212: Implement Bridge heartbeat and reconnection.
- [ ] WM-0213: Implement event sequencing, cursor, and duplicate event deduplication.
- [ ] WM-0214: Implement process CPU / RSS / FD collection.
- [ ] WM-0215: Implement Bridge queue length and callback lag metrics.
- [ ] WM-0216: Implement the KBEngine adapter conformance test.
- [ ] WM-0217: Implement an end-to-end login load test sample.
- [ ] WM-0218: Implement Bridge / Supervisor / bot fault injection tests.

## Phase 3: Native Rust Runtime

- [x] WM-0301: Implement the Actor state machine. *(1826473；crates/wm-runtime/src/actor.rs — Idle→Scheduled→Running→Waiting→Closed/Failed，审计迁移 + 单调时间戳；6 unit tests)*
- [x] WM-0302: Implement the Clock trait and ManualClock. *(5ca3078；crates/wm-runtime/src/clock.rs — Clock trait（now_nanos/is_deterministic）+ ManualClock（advance/set，回拨 panic）+ RealClock；5 unit tests；ManualClock 为全部确定性测试的时间基)*
- [x] WM-0303: Implement RealClock. *(5ca3078；clock.rs RealClock（Instant 基准，非确定性）；5 unit tests 含 RealClock 单调递增)*
- [x] WM-0304: Implement the Timer Scheduler. *(1826473；crates/wm-runtime/src/scheduler.rs — BinaryHeap 最小堆 + 墓碑取消；复用 benches/timer_scheduler.rs 基线设计；9 unit tests 含大规模正确性)*
- [x] WM-0305: Implement Seeded RNG. *(5ca3078；crates/wm-runtime/src/rng.rs — SplitMix64 + for_stream(seed,name) FNV-1a；10 unit tests 含确定性重放)*
- [x] WM-0306: Implement the Arrival Scheduler. *(7ed7804；crates/wm-runtime/src/arrival.rs — NHPP 精确逆变法 Λ⁻¹（ramp λ₀e^{kt}→hold 闭式），ceil 到 ns 保严格单调；11 unit tests，对拍 docs/spec/spectrum-semantics.md §7 解析 Λ(T)=3805.4)*
- [x] WM-0307: Implement Actor lifecycle management. *(7ed7804 + e448bbf 重构；crates/wm-runtime/src/lifecycle.rs — spawn/fire_due/start/suspend/resume/requeue/close/fail/drain + planned_to_start 偏差与峰值 runnable 测量；11 unit tests)*
- [x] WM-0308: Implement the Session state machine. *(11b3567；crates/wm-runtime/src/session.rs — Planned→Scheduled→Creating→Connecting→Authenticating→Active→Draining→Closed，any→Failed；审计迁移；4 unit tests)*
- [x] WM-0309: Implement the Transport trait. *(d6cd9aa；crates/wm-runtime/src/transport.rs — poll 式 send/recv/is_open/close + TransportError{WouldBlock,Closed,Io} + MockTransport（全双工对、full-close 语义）；5 unit tests)*
- [x] WM-0310: Implement the first TCP Transport. *(df858a7；crates/wm-runtime/src/tcp.rs — std-only 非阻塞 TcpStream 适配（无 tokio），connect/connect_timeout，WouldBlock→Ok(0)；4 unit tests 含 loopback 回声)*
- [x] WM-0311: Implement Framing. *(d6cd9aa；crates/wm-runtime/src/framing.rs — [u32 BE len][payload]，LengthPrefixed 有状态解码器（半帧缓冲）；8 unit tests)*
- [x] WM-0312: Implement Schema / Codec separation. *(d6cd9aa；crates/wm-runtime/src/codec.rs — Schema trait（VERSION/is_known）与 Codec 分离，wire [len][id][payload]；8 unit tests)*
- [x] WM-0313: Implement the Behavior trait. *(e448bbf；crates/wm-runtime/src/behavior.rs — 对象安全 Behavior + BehaviorContext（now_ns/rng/send/arm_timer/log）+ ActorDecision/BehaviorEffect；4 unit tests；Lua 不在 v1)*
- [x] WM-0314: Implement message event dispatch. *(e448bbf；crates/wm-runtime/src/dispatch.rs — EventDispatcher（subscriptions id→Vec<ActorId>，每 actor VecDeque 事件）；6 unit tests)*
- [x] WM-0315: Implement batch Actor scheduling. *(e448bbf + fa6f25e（缺陷修复）；crates/wm-runtime/src/batch.rs — ActorRunner（fire_arrivals + 行为 timer + ≤batch_size run 每 tick，路由 effects）；6 unit tests；**WM-0319 暴露并修复真实缺陷**：behavior 返回 Wait 时若有更多事件已入队则被搁置卡死，现 Wait 分支与 Continue 一致（有 pending 则 requeue，否则 suspend），50k 档修复前无法跑完）*
- [x] WM-0316: Implement Worker-local Metrics. *(45e654d；crates/wm-runtime/src/metrics.rs — 每 worker 无锁 AtomicU64 计数器/时长轨 + MetricsSnapshot + aggregate()（和 total、max maxima），覆盖 timer/actor/消息字节/run/lag/expiry/planned-to-start/runnable 长度；4 unit tests)*
- [x] WM-0317: Implement HDR histogram and V2 blob. *(77cdb06；crates/wm-runtime/src/histogram.rs — HistogramRecorder 包 wm_core::hdr（frozen_v1：2 位有效数字、ns、1..3.6e12），merge 经 record_value_count，encode_v2_blob/from_v2_blob；4 unit tests)*
- [x] WM-0318: Implement coordinated omission correction. *(4550012；crates/wm-runtime/src/omission.rs — record_with_expected_interval（Gil Tene：value 后补 value−interval…≥interval）；5 unit tests)*
- [x] WM-0319: Implement 5k / 20k / 50k benchmarks. *(403a06b；crates/wm-runtime/benches/runtime.rs — 真实 ActorRunner 端到端 lifecycle（5k/20k/50k × 每 actor 4 timer）+ start 路径；实测见 docs/benchmarks/runtime-benchmark.md（lifecycle 652/500/410 Kelem/s、start 721/351/235 Kelem/s，本机 ~2–5 倍负载）；本机可跑 50k，非假数字)*
- [x] WM-0320: Implement the 24h soak test. *(f26afba；crates/wm-runtime/tests/soak.rs — 重复 swell 周期模式（每周期独立 ActorRunner，内存上界=周期活集）；soak_short_repeated_cycles CI 内跑；soak_long_wallclock 为 24h 目标 `#[ignore]` 按需跑；实测 50k 两窗口（1.7 / 0.33 cycles/s，RSS 平坦 ~45 MiB）见 docs/benchmarks/soak-capacity.md——**完整 24h 连续运行未在本会话执行**（墙钟时间问题，非能力问题），已如实标注)*
- [x] WM-0321: Implement the Native Backend conformance test. *(1d4bc95；crates/wm-core/tests/backend_conformance.rs — 7 tests：穷举 12×12 迁移表对拍 docs/backends.md（27 条允许边恰配）、每迁移必带 backend_id/swell_id/state/observed_at/reason_code/detail/generation、失败/恢复路径、stale-generation 拒绝、Stopped 终态；**修复契约缺口**：TransitionRecord 原缺 swell_id，现机器绑定并逐条记录；worker 侧 apply_plan 绑定 swell（wm-worker 1 test）)*

## Phase 4: Basin and Unified Observability

- [ ] WM-0401: Implement the Basin data model.
- [ ] WM-0402: Implement the service topology model.
- [ ] WM-0403: Implement the Metric Catalog.
- [ ] WM-0404: Implement the Prometheus Provider.
- [ ] WM-0405: Implement Metric Mapping.
- [ ] WM-0406: Implement the Query Scheduler.
- [ ] WM-0407: Implement source_timestamp / observed_at / collected_at.
- [ ] WM-0408: Implement missing-data and delay detection.
- [ ] WM-0409: Implement Preflight.
- [ ] WM-0410: Implement observer_baseline.
- [ ] WM-0411: Implement observation overhead evaluation and degradation.
- [ ] WM-0412: Implement Generator Metrics aggregation.
- [ ] WM-0413: Implement Server Metrics aggregation.
- [ ] WM-0417: Implement report throughput metrics (QPS / TPS). *(用户 2026-11-08 问「报告是否含 qps/tps」——当前无。qps=请求/秒、tps=事务或消息/秒；服务端口径取自 Basin `server_performance.metrics`，客户端口径取自 `client_performance.per_backend`；输出含时间序列曲线 + 峰值 + p50/p95/p99；验收=跑一次 swell 后 `wm report` 导出的 JSON/Markdown 含 qps/tps 且与实际压测量级一致)*
- [ ] WM-0414: Implement timeline alignment.
- [ ] WM-0415: Implement data quality labels in reports.
- [ ] WM-0416: Implement end-to-end correlation reports.

## Phase 4.5: Assertion Provider（新增能力方向 / New Capability Direction）

<!-- 业务正确性断言：Wavemaker 只做调度与登记，业务语义由外部 Provider 判定。设计见 docs/spec/assertion-provider.md 与 docs/adr/ADR-007-assertion-provider.md。 -->

- [ ] WM-0701: Freeze the Assertion Provider contract (Execute/AssertionResult, pass|fail|inconclusive) and the Swell assertions config schema. *(design closed in docs/spec/assertion-provider.md §3 + ADR-007; schema/extensions landed in schemas/report/v1.json)*
- [ ] WM-0702: Implement assertion registration and execution-window scheduling (after_scenario / after_recovery / periodic).
- [ ] WM-0703: Implement the Provider transport and timeout/inconclusive handling.
- [ ] WM-0704: Implement failure evidence archiving (`<swell_dir>/evidence/...`).
- [ ] WM-0705: Implement report assertions-section generation and Test Validity degradation (R5) end-to-end.

## Phase 5: Distributed Execution and Recovery

- [ ] WM-0501: Implement the Worker Registry.
- [ ] WM-0502: Implement the Backend Registry.
- [ ] WM-0503: Implement resource capacity descriptions.
- [ ] WM-0504: Implement ShardPlan.
- [ ] WM-0505: Freeze the Worker count at Swell creation.
- [ ] WM-0506: Implement leases and heartbeats.
- [ ] WM-0507: Implement new-admission freezing on lost contact.
- [ ] WM-0508: Implement Backend generation.
- [ ] WM-0509: Implement recovery handshake and state synchronization.
- [ ] WM-0510: Implement multi-Worker Metrics aggregation.
- [ ] WM-0511: Implement cross-Worker HDR merging.
- [ ] WM-0512: Implement Controller restart recovery.
- [ ] WM-0513: Implement Worker fault injection.
- [ ] WM-0514: Implement multi-Backend mixed Swells.
- [ ] WM-0515: Implement distributed 24h soak.

## Phase 6: Dashboard and Extension Ecosystem

<!-- 仅在 v1 产品形态需要时启动 / Only start when the v1 product form requires it. -->

- [ ] WM-0600: Decide the desktop surface (native shell + web view vs web-only). *(用户 2026-11-08 问「桌面界面没有设计」——先拍板形态再启动 Phase 6；候选：Tauri(原生壳+WebView)/Electron/纯 Web 仪表盘；推荐 Tauri——体积小、Rust 后端可直接调 controller)*
- [ ] WM-0601: Implement Swell list and detail views.
- [ ] WM-0602: Implement Backend / Worker status pages.
- [ ] WM-0603: Implement Spectrum editing and validation display.
- [ ] WM-0604: Implement the load timeline.
- [ ] WM-0605: Implement Generator / Server metric curves.
- [ ] WM-0606: Implement report comparison.
- [ ] WM-0607: Implement users and permissions.
- [ ] WM-0608: Implement audit logs.
- [ ] WM-0609: Evaluate Docker / Podman launchers.
- [ ] WM-0610: Evaluate a Lua Behavior SDK.
- [ ] WM-0611: Evaluate Go / C++ / Python third-party SDK completeness.

## 审核发现

- [x] 审核发现：被拒绝的 ApplyPlan 请求仍先覆盖 swell 绑定，导致之后的状态记录归属错误（crates/wm-worker/src/backend.rs:233）— 属实，修于 7673f08（合法性检查前置到 bind_swell 之前），测试 `rejected_apply_plan_does_not_overwrite_swell_binding` 绿。
- [x] 审核发现：提前 drain 尚未到达的 actor 时，其 Scheduled 时间戳被设为未来的计划时间，导致单调时间断言触发并崩溃（crates/wm-runtime/src/lifecycle.rs:71）— 属实，修于 e9f208a（spawn 以观测时刻戳记、启动定时器按计划时刻），测试 `draining_a_not_yet_arrived_actor_keeps_history_monotonic` 绿。
- [x] 审核发现：actor 关闭后未取消订阅且到期定时器仍会投递事件，后续消息和定时器会在无法运行的 actor 队列中持续累积（crates/wm-runtime/src/batch.rs:177）— 属实，修于 bb38f63（Close 分支取消订阅+清空+取消定时器），测试 `closed_actor_drops_subscriptions_and_pending_timers` 绿。
- [x] 审核发现：发送结果仅包含消息 ID 和载荷而丢失来源 actor，多个会话同时发送时无法将消息路由到各自的传输连接（crates/wm-runtime/src/batch.rs:193）— 属实，修于 12dcd6a（TickOutcome::sends 携带 `(ActorId,u32,Vec<u8>)`），测试 `sends_carry_the_source_actor` 绿。
- [x] 审核发现：planned-to-start 偏差在 actor 进入运行队列时计算而非行为实际执行时计算，批处理积压期间会低估启动延迟（crates/wm-runtime/src/lifecycle.rs:103）— 属实，修于 d0e2047（`record_first_run` 在执行时测量），测试 `planned_to_start_deviation_measures_actual_execution` 绿。
- [x] 审核发现：启动基准把进入队列的 actor 计为已执行，超过批次大小时首个 tick 即结束，未测量剩余 actor 的启动行为（crates/wm-runtime/benches/runtime.rs:123）— 属实，修于 84392d5（以 `out.run` 排空并断言每个 actor 恰好执行一次），文档修订于 967373d（docs/benchmarks/runtime-benchmark.md §3 说明语义变更）。
- [x] 审核发现：取消未知的未来定时器 ID 会留下墓碑并静默取消后来分配相同 ID 的定时器，已触发 ID 的取消还会永久累积墓碑（crates/wm-runtime/src/scheduler.rs:97）— 属实，修于 84392d5（`live_seqs` 镜像仅墓碑活跃 ID），测试 `cancelling_an_unallocated_id_does_not_kill_a_later_timer` 与 `cancelling_fired_and_unknown_ids_keeps_tombstone_set_empty` 绿。
- [x] 审核发现：超过直方图上限的延迟被静默丢弃而非截断或报告，长尾样本消失会低估延迟和计数（crates/wm-runtime/src/histogram.rs:41）— 属实，修于 f3a560c（钳位到 frozen-v1 1h 上限而非丢弃），测试 `above_ceiling_samples_are_clamped_not_dropped` 绿。
- [x] 审核发现：TCP 对空缓冲区的读取返回零也被当作对端关闭，调用 recv 空切片即可错误地关闭仍然活跃的连接（crates/wm-runtime/src/tcp.rs:64）— 属实（跨平台潜在缺陷），修于 e03eb8c（空缓冲区读取短路为 no-op 而非 EOF），测试 `recv_with_empty_buffer_does_not_close_a_live_connection` 绿；本 Linux 非阻塞空读返回 EAGAIN 掩盖了该路径，保护消除平台依赖。
- [x] 审核发现：WorkerMetrics 没有接入调度器、行为执行或传输路径，实际运行时不会自动记录所需的调度和消息指标（crates/wm-runtime/src/metrics.rs:17）— 属实，修于 82d1d33（ActorRunner 内嵌 WorkerMetrics，接通 timer 增删/触发、actor 启动/关闭、消息收发与字节、planned-to-start、runnable 队列 gauge；run_ns 与 event-loop lag 属实时 worker 层，代码内注明），测试 `metrics_track_the_tick_paths` 绿。
- [x] 审核发现：到达时间反演使用 ln(1+x) 导致极小间隔因浮点舍入归零，允许的高到达率下会产生重复时间戳并在调试构建触发断言（crates/wm-runtime/src/arrival.rs:125）— 属实，修于 3230e25（`ln_1p` + `next_arrival_after` 严格前进钳位），测试 `inverse_cumulative_is_strictly_monotone_for_tiny_increments` 绿。

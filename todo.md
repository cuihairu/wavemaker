# Wavemaker Todo

Task checklist from the phased implementation plan (Phase 0–6). Keep WM task IDs stable.

## Phase 0: Semantic Closure and Technical Verification

- [x] WM-0001: Pin down the precise definitions of Mode A/B/C in the original architecture document. *(2026-11-09 冻结于 docs/architecture.md「Mode A/B/C Definitions」+ docs/zh/architecture.md 1.4 节镜像：以「谁建 Bot / 谁持 socket / 控制观测粒度」三问钉死三模式；Mode A 能力主张引用 docs/spec/kbengine-api-audit.md 的 file:line 证据（addBots 批次累加无单控 bots.cpp:344-388；KBEngine.bots 只读映射 pybots.cpp:16-113）；原始架构文档不在本地，定义以计划书 §6.6 + 已审计源码事实为准，若原始文档重现以新 ADR 调和；ADR-005 D2 的 Mode B「待确认」行已更新为已冻结、v1 不实现、实现前需单独 ADR)*
- [x] WM-0002: Define the domain model for Swell, Spectrum, Basin, Backend, Worker, Actor. *(docs/glossary.md)*
- [x] WM-0003: Close the arrival / concurrency / lifetime semantics. *(docs/spec/spectrum-semantics.md)*
- [x] WM-0004: Define saturation policies. *(docs/spec/spectrum-semantics.md §5)*
- [x] WM-0005: Verify concurrency curves for exponential arrival under different lifetime distributions. *(tools/spectrum_sim.py + tools/test_spectrum_sim.py; results in docs/spec/spectrum-semantics.md §7)*
- [x] WM-0006: Define plan-as-data and ShardPlan freezing rules. *(docs/adr/ADR-006-plan-as-data.md)*
- [ ] WM-0007: Fix HDR V2 blob parameters and cross-implementation conformance tests.
- [x] WM-0008: Inspect the target KBEngine version's bot API and callback source code. *(static audit closed: docs/spec/kbengine-api-audit.md — file:line evidence at fork commit bf1a9cd6; 7 plan-vs-source discrepancies logged; runtime items deferred to WM-0009/WM-0010 checklist)*
- [ ] WM-0009: Verify that the KBEngine Bridge can execute commands safely in the engine scripting context. *(2026-11-09 静态闭合：§8 清单 8/8 项静态验证完成（S1–S8，docs/spec/kbengine-api-audit.md §8.1，基线 fork 3082ab50）——单线程、阻塞影响面、异常吞噬、telnet 无 GIL 竞争、ScriptID 复用、onInit 单次、bots 映射快照、addBots 节拍（推翻 bots.h 注释）全部有 file:line 证据；Phase 2 实现约束已导出（§8.3）。残留 RT2/RT3 属运行时，BLOCKED：Bridge 未实现（Phase 2 WM-0201 起）且本机无运行中的 bots 进程/集群；Bridge 落地时按 §8.3 约束实现并复测)*
- [ ] WM-0010: Measure KBEngine callback scheduling granularity and Bridge queue latency. *(静态部分已闭合：粒度=1/gameUpdateHertz 默认 100ms 量化下界（audit §3）；addBots 到达精度上界≈1s 批+gameTick（§8.1 S8）。实测（RT1/RT4）BLOCKED：需运行中的 KBEngine 集群+bots 进程，本机 kbengine-build 仅存构建产物无部署、无 Bridge 被测对象；不虚标数字)*
- [x] WM-0011: Build a minimal Rust Worker timer benchmark. *(crates/wm-core/benches/timer_scheduler.rs (criterion); baseline results in docs/benchmarks/timer-benchmark.md — load-sensitive, relative comparisons only)*
- [ ] WM-0012: Decide whether v1 needs a resident Dashboard and multi-user management.
- [x] WM-0013: Freeze Protobuf package names, version compatibility policy, and error code conventions. *(docs/spec/protobuf-conventions.md; error-code skeleton, field freeze in Phase 1 WM-0103)*
- [ ] WM-0014: Clarify the persistence boundary for Swell data.

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

- [ ] WM-0201: Create the KBEngine Bridge Python bootstrap.
- [ ] WM-0202: Implement the Bridge local IPC client.
- [ ] WM-0203: Implement Handshake and capability report.
- [ ] WM-0204: Implement Supervisor startup of KBEngine Bots.
- [ ] WM-0205: Implement Swell / Backend / generation injection.
- [ ] WM-0206: Implement the command queue and engine callback scheduling.
- [ ] WM-0207: Implement ApplyPlan batch plan mapping.
- [ ] WM-0208: Implement bot count collection.
- [ ] WM-0209: Implement connection and login event collection (subject to APIs available in the target version).
- [ ] WM-0210: Implement StopAdmission.
- [ ] WM-0211: Implement Drain / Shutdown.
- [ ] WM-0212: Implement Bridge heartbeat and reconnection.
- [ ] WM-0213: Implement event sequencing, cursor, and duplicate event deduplication.
- [ ] WM-0214: Implement process CPU / RSS / FD collection.
- [ ] WM-0215: Implement Bridge queue length and callback lag metrics.
- [ ] WM-0216: Implement the KBEngine adapter conformance test.
- [ ] WM-0217: Implement an end-to-end login load test sample.
- [ ] WM-0218: Implement Bridge / Supervisor / bot fault injection tests.

## Phase 3: Native Rust Runtime

- [ ] WM-0301: Implement the Actor state machine.
- [ ] WM-0302: Implement the Clock trait and ManualClock.
- [ ] WM-0303: Implement RealClock.
- [ ] WM-0304: Implement the Timer Scheduler.
- [ ] WM-0305: Implement Seeded RNG.
- [ ] WM-0306: Implement the Arrival Scheduler.
- [ ] WM-0307: Implement Actor lifecycle management.
- [ ] WM-0308: Implement the Session state machine.
- [ ] WM-0309: Implement the Transport trait.
- [ ] WM-0310: Implement the first TCP Transport.
- [ ] WM-0311: Implement Framing.
- [ ] WM-0312: Implement Schema / Codec separation.
- [ ] WM-0313: Implement the Behavior trait.
- [ ] WM-0314: Implement message event dispatch.
- [ ] WM-0315: Implement batch Actor scheduling.
- [ ] WM-0316: Implement Worker-local Metrics.
- [ ] WM-0317: Implement HDR histogram and V2 blob.
- [ ] WM-0318: Implement coordinated omission correction.
- [ ] WM-0319: Implement 5k / 20k / 50k benchmarks.
- [ ] WM-0320: Implement the 24h soak test.
- [ ] WM-0321: Implement the Native Backend conformance test.

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

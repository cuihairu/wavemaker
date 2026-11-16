# Swell 数据持久化边界（WM-0014）

> 状态：v1 冻结（2026-11-09）。实现基准：`crates/wm-core/src/store.rs`（`SwellStore`，WM-0110 落地，11 unit tests）。
> 来源：计划书 §7「数据存储」、docs/tech-stack.md「Is a Database Required?」/「元数据与实验数据分离」、ADR-007 / assertion-provider §6。

## 1. 边界总览

一次 Swell 产生的数据分四类，持久化责任不同：

| 数据类 | 内容 | 持久化责任 | v1 落点 |
|---|---|---|---|
| 元数据 | swell id、名称、phase、谱面引用、时间戳 | **Wavemaker 拥有**，结构化 JSON | `<root>/<swell_id>/metadata.json` |
| 事件时间线 | 阶段边界、控制动作、校验结论（docs/swell.md：阶段边界必须写入时间线） | **Wavemaker 拥有**，append-only JSONL | `<root>/<swell_id>/timeline.jsonl` |
| 实验数据（evidence） | 断言证据、崩溃转储、逐后端原始产物 | **目录由 Wavemaker 预创建，内容由 Provider 写**；store 从不解释内容 | `<root>/<swell_id>/evidence/<assertion-name>/<timestamp>/` |
| 服务端时序（Basin） | 被测系统的指标时间序列 | **外部 Provider 拥有**（Prometheus 等）；报告引用查询结果，不落 swell 目录 | Basin（Phase 4） |

派生工件（`wm report` 导出的 JSON/Markdown、直方图聚合）是**报告时从上述数据派生**的输出，不作为 Swell 的持久化边界成员；报告工件写到哪里由调用方决定。

## 2. SwellStore 的行为约定（v1 冻结）

1. **布局**：`<store_root>/<swell_id>/` 一目录一 Swell；`metadata.json` 与 `timeline.jsonl` 直接位于 swell 目录（小、结构化），一切实验数据在 `evidence/` 子树之下（元数据与实验数据分离，tech-stack）。
2. **原子性**：`metadata.json` 以 temp 文件 + rename 覆盖写，读者永远看不到撕裂状态；`timeline.jsonl` 追加写。阶段转换 = 重写 metadata + 追加一条 `phase` 时间线事件（docs/swell.md 阶段边界入时间线）。
3. **路径安全**：swell id 与断言名必须是单个安全路径段（非空、无分隔符、非 `.`/`..`、无空白），id 永远无法逃出 store 根。
4. **evidence 约定**：`<swell_dir>/evidence/<assertion-name>/<timestamp>/`，由 store 按请求预创建（ADR-007 §6：controller 预创建、Provider 写入、store 不解释内容）；时间戳目录名 `<unix-seconds>-<nanos>`，字典序即时间序。
5. **无数据库**：CLI 驱动的单次 Swell 不需要常驻数据库（tech-stack）；`SwellStore` 是文件系统 + JSON 的薄封装，重开即可恢复（tests: `timeline_and_metadata_survive_reopen`）。

## 3. 明确不在 v1 持久化边界内的东西

- **Worker 运行时指标**（WM-0316/0317）：Worker 常驻内存（计数器 + HDR 直方图），在报告/聚合阶段才序列化为 V2 blob（`wm-core::hdr`）进入派生工件；v1 不逐请求落盘。
- **Basin 时序**：永远留在 Provider 侧；报告只携带引用（查询窗口、source_timestamp 对齐信息，Phase 4）。
- **逐请求事件流**：v1 不落盘逐请求明细；延迟分布以 HDR 直方图（可合并、可跨 Worker 汇总，generator-observability）为持久化粒度。
- **计划（Spectrum / ShardPlan）**：以 plan-as-data 文件形态存在（ADR-006），由调用方指定路径，`metadata.json` 只存引用（`spectrum_path`）。

## 4. 与其他组件的边界

- **Controller**：v1 单机形态下 Controller 与 CLI 共用 `SwellStore`；分布式形态（Phase 5）引入远端 Worker 后，swell 目录仍归 Controller 所在主机所有，Worker 不直接写 store（经控制面上报）。
- **Bridge / Supervisor**：进程级产物（崩溃、异常退出记录）走 evidence 约定子树；不写 metadata/timeline（这两个文件的写入方只有 store API）。
- **Assertion Provider**：见 ADR-007 / assertion-provider §6——Provider 只写自己的 evidence 目录，超时/不确定结果也不得写时间线（时间线事件由 Controller 登记）。

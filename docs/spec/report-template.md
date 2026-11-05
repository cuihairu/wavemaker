# Swell Report Template（报告格式模板 v1）

> 状态：Phase 0 草案
> 来源：计划书 §13.2（14 节报告结构）、§13.3（报告结论约束）、§11.3（数据质量标签）、§16.3（测试有效性）。
> 数据交换格式：`schemas/report/v1.json`（JSON Schema draft 2020-12）；校验：`python3 tools/validate_report.py <report.json>`；单测：`python3 -m pytest tools/test_validate_report.py`（24 passed）；样例：`schemas/report/examples/sample-v1.json`。
> 扩展：第 15 节 `assertions`（业务正确性断言，ADR-007，可选节）——设计见 `docs/spec/assertion-provider.md`。

## 0. 通用约定

### 数据源标签（每个 section 必填 `sources`）

| 标签 | 含义 |
|---|---|
| `Planned` | Controller 冻结的计划数据（ShardPlan / Spectrum） |
| `Actual` | Generator 实际执行数据（到达、并发、延迟直方图） |
| `Basin` | 被测服务端观测数据（Prometheus 等 Provider） |
| `Generator` | 压测发生器自身健康数据（CPU/RSS/队列/调度延迟） |

### 数据质量标签（每个 section 必填 `quality`；计划书 §11.3 全集）

`Complete` / `Partial` / `Delayed` / `Missing` / `Estimated` / `Invalid` / `GeneratorSaturated` / `ObserverDegraded`。**缺失数据不能默认为零**（§11.3）；缺失以 `Missing` 标注，数值字段一律允许 `null`，报告不得用 0 冒充缺失。

### 数据质量标签标注方式（映射规则，建议）

- 数据完整且时间戳连续 → `Complete`。
- 覆盖存在但有空窗/采样间隔不满足需求 → `Partial`。
- 数据迟到超过查询窗口 → `Delayed`。
- 区间无数据 → `Missing`。
- 由插值/模型推算 → `Estimated`。
- 校验失败（单位/类型/摘要不一致）→ `Invalid`。
- 发生器达到资源瓶颈（§11.1）→ `GeneratorSaturated`。
- 观测降级（observer_baseline 超阈值自动降级，§10.6）→ `ObserverDegraded`。

## 1. 14 节报告结构（§13.2）逐节定义

| # | Section（schema key） | 核心数据字段 | 单位 | 图表类型 | 数据源 |
|---|---|---|---|---|---|
| 1 | `experiment_summary` | swell_name, started_at, ended_at, backend_count | — | 摘要表 | Planned/Actual |
| 2 | `spectrum_load_profile` | arrival_model, lifetime_distribution, lifetime_mean_s, concurrency_target, concurrency_max, saturation_policy | s、count | 配置表 + 到达速率曲线 | Planned |
| 3 | `planned_vs_actual_arrival` | planned_arrivals, actual_arrivals, rejected_arrivals, deviation_p50_s, deviation_p99_s | count、s | 双曲线对比 + 偏差直方图 | Planned/Actual |
| 4 | `concurrency_curve` | peak_concurrency, series_points（活跃量时间序列） | count | 时间序列折线 | Actual（对照 Planned 预测，见 spectrum-semantics.md） |
| 5 | `client_performance` | per_backend[]: latency_p50/p95/p99_ms, effective_request_rate, connections | ms、req/s、count | 分 Backend 直方图/折线；**禁止跨 Backend 平均 P99**（§11.2） | Actual |
| 6 | `generator_health` | saturation{flag,reason}, workers[]: cpu_percent, memory_rss_bytes, event_queue_max, scheduler_lag_p99_ms, restarts | %、bytes、count、ms | 时间序列 + 阈值线 | Generator |
| 7 | `engine_bot_backend_details` | adapters[]: adapter_name, scheduling_resolution_ms, bot_count_total/connected/logged_in, capabilities_note | ms、count | 分组表 + 状态堆叠图 | Generator/Actual |
| 8 | `server_performance` | metrics[]: metric, unit, coverage_percent | 按 Metric Catalog 统一单位（§10.3） | 时间序列折线（与负载曲线对齐，§10.4） | Basin |
| 9 | `correlation_analysis` | claims[]: metric_a, metric_b, coefficient, asserts_causality(恒 false) | — | 散点/相关矩阵 | Basin+Actual+Generator |
| 10 | `bottleneck_analysis` | claims[]: resource, statement, single_metric_inference(恒 false), evidence[] | — | 带证据引用的结论表 | Basin+Generator+Actual |
| 11 | `data_quality` | labels_summary：8 种标签的计数 | count | 标签分布表 + 时间轴覆盖图 | 全部 |
| 12 | `test_validity` | verdict(Valid/Invalid/Partial), failed_criteria[] | — | 判定表（判定条件见 §16.3） | Planned/Actual/Generator |
| 13 | `environment` | versions{}, configuration_digest（= plan_digest，§8.4） | — | 版本矩阵表 | Planned |
| 14 | `raw_data_references` | artifacts[]: path, format(parquet/json/hdr-blob-v2/log/evidence-bundle) | — | 引用清单 | 全部 |
| 15 | `assertions`（可选，ADR-007） | results[]: provider, name, status(pass/fail/inconclusive), checked_at, trigger, message, evidence[] | — | 断言结果表 + 证据引用 | AssertionProvider |

### HDR 汇总约束（§11.2，影响第 5/8 节）

- 直方图参数（sigfigs/lowest/highest/epoch）由 Controller 下发并写入计划元数据；V2 histoblob 以 opaque bytes 传输时必须携带 schema/version/参数摘要。
- 不允许直接平均各 Worker 的 P99；使用可合并直方图汇总总体分布。

### 时间戳约束（§10.4，影响第 8/9 节）

保留 source_timestamp / observed_at / collected_at 三类时间戳；缺失区间显式标注，时钟偏移记录估计值；形状对齐不得用于伪造时间精度。

## 2. §13.3 结论约束在 schema 中的字段级落点

| 计划书约束 | Schema 强制（`schemas/report/v1.json`） | 单测 |
|---|---|---|
| 发生器饱和时，不应给出服务端最大容量结论 | R1/R1b：`generator_health.saturation.flag == true` 或 `overall_data_quality == "GeneratorSaturated"` 时 `conclusions.capacity_conclusion` 必须为 `null` | test_generator_saturation_forbids_capacity_conclusion 等 2 例 |
| 客户端延迟不能替代服务端 Tick 指标 | R2：`capacity_conclusion` 非空时 `server_performance.sources` 必须含 `Basin` 且 quality 不得为 Missing/Invalid；`capacity_conclusion.based_on` 枚举排除 `Generator` | test_capacity_conclusion_requires_basin_server_data |
| 监控覆盖不足时，必须降低结论置信度 | R3：`server_performance.quality ∈ {Partial, Delayed, Missing, Invalid, ObserverDegraded}` 时 `conclusions.confidence` 必须为 `"low"` | test_poor_observability_forces_low_confidence |
| 指标曲线相关不等于因果 | correlation/bottleneck/conclusions 的 `asserts_causality` 均 `const false` | test_correlation_may_not_assert_causality |
| CPU 高不自动证明 CPU 是唯一瓶颈 | bottleneck claim 的 `single_metric_inference` `const false`，且 `evidence` minItems=1 | test_single_metric_bottleneck_inference_rejected |
| 不同 Backend 结果应可分组查看 | R4：`experiment_summary.backend_count ≥ 2` 时 `client_performance.per_backend` minItems=2 | test_multiple_backends_require_per_backend_grouping |
| 业务断言失败时测试不能被渲染为有效（ADR-007 §5） | R5：`sections.assertions.results` 中任一 `status == "fail"` 时 `test_validity.verdict` 必须为 `Invalid` 或 `Partial` | test_failed_assertion_forbids_valid_verdict 等 8 例 |

## 3. 验证方式

```bash
# 校验一份报告
python3 tools/validate_report.py schemas/report/examples/sample-v1.json

# schema + 约束单测（样例通过、坏样例拒绝）
python3 -m pytest tools/test_validate_report.py    # 24 passed
```

## 4. 遗留与待确认

- 报告的 HTML/Parquet 渲染属 Phase 4（WM-0416 报告数据与 WM-0415 质量标签）；本模板只冻结数据交换格式。
- Metric Catalog 的统一单位目录未建立前，`server_performance.metrics[].unit` 为自由字符串，Catalog 落地后改为受控枚举（待验证）。
- 3% 观测开销阈值的测量方法计划书明示「需定义」（§10.6），ObserverDegraded 的触发判定依赖该定义。

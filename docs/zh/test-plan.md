---
title: 测试计划
---

# 测试计划（Test Plan）

测试计划是对一次负载验证的声明式描述：一个有序的步骤列表（注入负载、
保持、采集、断言），由执行器端到端驱动——逐条判定断言，并写出可逐行
对比的运行目录与 Markdown 报告。

计划文件是 TOML，校验沿用 Spectrum 的可解释违例风格。模型位于
`crates/wm-core/src/testplan.rs`（纯模型，零 IO）；执行器是
`wm test run`（`crates/wm-cli/src/runner.rs`）。

## 快速开始（stub 后端）

```bash
cargo build -p wm-cli
./examples/run_demo.sh                          # 三个示例计划全部真跑
wm test run examples/plans/smoke.toml --time-scale 0   # 单个计划，瞬时完成
```

没有真实游戏服务器时，默认后端是进程内 **stub** 模拟器：负载曲线来自
计划引用的真实 spectrum 文件（经 wm-runtime 的 `ArrivalPlan`——与原生
运行时执行的是同一套到达率数学），但指标是合成公式，且每份报告都醒目
标注 `simulated`。`--time-scale 0` 把真实睡眠压缩为零、模拟时间照常全额
推进；`1.0` 为真实时间。

三个示例计划在 `examples/plans/`：

| 计划 | 形状 | 断言 |
|---|---|---|
| `smoke.toml` | 1 并发，5 分钟 | 探活可达、`error_rate = 0`、p99 < 250 ms |
| `baseline.toml` | 阶梯 10 → 50 → 100，每级 2 分钟 | 最差级 p99 < 250 ms、零错误 |
| `soak.toml` | 5 并发，30 分钟 | 内存增长 < 15 MB/h（两快照趋势） |

预期报告样例（真实 stub 运行的产物）在同目录 `examples/plans/expected/`。

## 计划结构

```toml
[meta]
name = "smoke"          # 必填；同时命名运行目录
game = "demo"           # 可选
engine = "stub"         # 可选

[[steps]]
id = "load"             # 必填，计划内唯一
action = "inject_load"
[steps.params]
spectrum = "../spectra/demo-ramp.yaml"
concurrency = 1         # 可选：覆盖 spectrum 的并发
duration_secs = 300     # 观测预算（见下）
expect_sha256 = "…"     # 可选：钉住 spectrum 内容哈希
```

未知参数键是解析错误；已知键放到不支持的 action 上报
`E_PARAM_NOT_APPLICABLE`。相对 `spectrum` 路径锚定在计划文件所在目录，
计划在任何工作目录下运行结果一致。顶层键（`continue_on_assert_fail`）
必须写在 `[meta]` 之前——表头之后的一切都归属该表。

### Action 一览

| Action | 参数 | 效果 |
|---|---|---|
| `prepare` | `target`、`timeout_secs`、`warmup_secs` | 探测目标可达性；不可达即中止。 |
| `inject_load` | `spectrum`、`concurrency`、`swell_id`、`expect_sha256`、`duration_secs` | 从 spectrum 文件启动负载；会先停掉上一个注入的负载（单活跃负载不变量）。 |
| `hold` / `cooldown` | `secs` | 等待（观测窗口 / 收尾间隔）。 |
| `step_load` | `spectrum`、`levels`、`step_secs`、`capture_per_level` | 阶梯：每级——释放、按该并发注入、等待、采集（快照带级索引）。 |
| `capture` | `label`、`source` | 取一份指标快照。 |
| `assert` | `metric`、`max` 和/或 `min`、`source` | 判定匹配的快照（见下）。 |
| `report` | `stage` | 向运行目录写中间报告 `report-<stage>.md`。 |

### `inject_load` 的时长预算语义

`duration_secs` 不是步骤内睡眠，而是观测预算：由后续的
`hold`/`cooldown` 消耗；预算耗尽时负载在计划中途被释放（事件线记录
`duration budget exhausted`）。这保证核心验证流——注入 → 保持 → 采集 →
断言——发生在负载存活期内。

## 断言

五个指标：

| 指标 | 边界方向 | 说明 |
|---|---|---|
| `latency_p99_ms` | `max`（上限） | 尾延迟。 |
| `error_rate` | `max`（上限） | 0–1 的比例。 |
| `throughput` | `min`（下限） | 最低可接受吞吐。 |
| `disconnect_recovery_secs` | `max`（上限） | 断连到恢复的时长。 |
| `memory_growth_mb_per_hour` | `max`（上限） | 泄漏趋势：对匹配快照的内存序列做最小二乘斜率（MB/h）；至少需要 2 个点。 |

断言按 `source`（产出步 id）匹配快照；缺省匹配全部快照。单值指标只要
有任一匹配快照越界即 FAIL（`observed` 报最差值）。

缺数据源永不算 PASS：无匹配快照、或快照缺该指标，判 **NotApplicable**，
缺口如实随计划呈现。例如 stub 没有断连，那里的
`disconnect_recovery_secs` 断言判 NA。

`continue_on_assert_fail = true`（计划级）让 FAIL 后继续执行剩余步骤；
默认 fail-fast。它只影响执行序——判定不受影响，任一 FAIL 都使总判定为
`failed`（CLI 退出码 4）。

## 后端

| 后端 | 参数 | 指标 | 说明 |
|---|---|---|---|
| `stub` | 默认 | 合成、确定 | 进程内模拟器；`--time-scale` 压缩真实睡眠（0–1）。报告一律带 simulated 横幅。 |
| `controller` | `--backend controller` | 需要 `--basin-url` | 经 REST 驱动真实控制器（run/stop）。未配 `--basin-url` 时 capture 具名报错引 WM-0412/WM-0413——控制器尚无指标来源，执行器不编造。 |

## 运行目录

每次运行写一个目录（`wm-runs/<name>-<timestamp>`，或 `--out`）：

| 文件 | 内容 |
|---|---|
| `plan.toml` | 计划原文。 |
| `events.jsonl` | 事件线（步骤开始/结束、负载注入/释放、断言判定……）。 |
| `metrics.json` | 全部采集快照。 |
| `asserts.json` | 逐断言判定（observed 与原因）。 |
| `run.json` | 结构化结果（backend、`time_scale`、plan sha256、各 spectrum 哈希、verdict）。 |
| `report.md` | 渲染报告（另有中间 `report-<stage>.md`）。 |

计划内容哈希与每个注入 spectrum 的 sha256 都会记录，同一计划的两次运行
可逐行 diff；`expect_sha256` 钉住 spectrum，负载内容被静默改动时计划
拒绝运行。

## MCP

同一引擎暴露给 AI agent：`run_test_plan` 执行计划（`failed` 判定以成功
结果返回并携带原因），`get_test_report` 回读已完成的运行——见
[crates/wm-mcp/README.md](https://github.com/cuihairu/wavemaker/blob/main/crates/wm-mcp/README.md)。

## 诚实边界

- stub 指标是模拟产物：模拟负载状态上的确定性公式。它们验证的是
  计划/断言管线，不是服务器。
- 真实指标等引擎侧聚合（WM-0412 / WM-0413）；controller 后端只从显式
  `--basin-url` 端点采集，未配置时大声报错。
- KBEngine 真机负载等 Bridge 执行路径（[KBEngine 适配器](/zh/kbengine)）；
  当前示例走的是原生编排面。

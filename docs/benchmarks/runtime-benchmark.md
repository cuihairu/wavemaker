# Native Runtime Benchmark（WM-0319，Phase 3）

> 状态：Phase 3 闭合材料（WM-0319；被测对象为真实批量调度器，非占位实现）
> 可执行验证：`cargo bench -p wm-runtime --bench runtime`
> 结论措辞约束：本机为多 agent 共享的高负载机器，以下数字**只用于相对比较**（规模之间的趋势），不构成任何绝对性能承诺。

## 1. 被测对象

`wm-runtime::batch::ActorRunner` 端到端（docs/native-runtime.md §"Scheduler" 的强制点）：

- **生命周期**：spawn N 个 actor（每 actor 在 Started 时 arm 4 个 timer，间隔 1 ms），时钟以 100 µs 步进，驱动到全部 actor 完成 4 次 timer 触发并 close——覆盖 start / timer-fire / resume / close 全链路；
- **start**：仅 spawn + 批量 start（无 timer），测调度路径吞吐。

调度点规模：5k / 20k / 50k Actor × 每 Actor 4 timer（= 20k / 80k / 200k 个 timer 事件）。`batch_size=1024`。

> 与 WM-0011（wm-core `timer_scheduler`，占位二叉堆基线）区分：本测的是**真实运行时全链路**；WM-0011 是 Phase 0 的 timer 微基线。两者不混比。

## 2. 负载上下文（重要）

测量时段 `/proc/loadavg`：`90.79 88.28 82.53`（14 CPU），即负载约为核数的 **~6 倍**——比 WM-0319 首次测量（22/39/69，约 1.6–5 倍）更重。本文件 2026-11-09 修订版记录的是**这一轮重负载下的快照**；吞吐数字受 CPU 抢餐主导，**任何绝对数字都不可外推**。

## 3. 吞吐结果（criterion 默认采样，2026-11-09 修订）

中位点估计（`thrpt` 区间中值）：

| 操作 | 5k Actor（20k timer） | 20k Actor（80k timer） | 50k Actor（200k timer） |
|---|---|---|---|
| lifecycle（端到端，每 actor 5 事件） | 137 Kelem/s（182.8 ms） | 253 Kelem/s（395.7 ms） | 225 Kelem/s（1112.4 ms） |
| start（每 actor 仅 start，执行到完成） | 178 Kelem/s（28.1 ms） | 97 Kelem/s（206.3 ms） | 123 Kelem/s（407.3 ms） |

> 修订说明（**重要**）：`start` 一行的语义在 2026-11-09 修正。旧版 `run_start` 以 `out.started`（`fire_due` 把到期 actor 移入 runnable 队列的波次）为完成判据，但 `batch_size=1024` 使单 tick 最多执行 1024 个 behavior——旧数字把「一批波次」当成了整规模，测量的是单 tick 而不是整个 start 路径。新版以 `out.run`（实际执行数）为判据，`assert` 每个 actor 恰好执行一次。**因此旧 `start` 数字（721/351/235 Kelem/s）与新版不可比**，仅本修订版有效。lifecycle 一行的测量判据未变（本就按 `total_closed` 排空）。

## 4. 趋势（本修订版负载下）

1. **端到端时间随规模单调上升**：lifecycle 182.8 → 395.7 → 1112.4 ms；start 28.1 → 206.3 → 407.3 ms。这是本修订版下唯一稳的趋势。
2. **元素吞吐在本轮负载下不再单调**（lifecycle 137→253→225，start 178→97→123 Kelem/s）：5k 档小迭代被负载尖峰显著放大，吞吐的绝对/跨档比较在此负载下无意义；**只可用时间趋势**。WM-0319 首次测量（轻负载 22/39/69）曾显示吞吐单调下降——那是该负载窗口下的趋势，不是普适规律。
3. **lifecycle 与 start 的相对关系**：50k 档 lifecycle 每 actor 5 事件 vs start 每 actor 1 事件，lifecycle 的 per-actor 成本被 timer 触发与 resume 摊薄，故单位时间完成的事件数更多（225 vs 123 Kelem/s）——与设计预期一致。

## 5. 已知边界与诚实标注

- **50k 全链路一次 ~1.1 s**（200k timer 事件，本轮 ~6 倍负载）：本机仍可稳定跑完 50k 操作点（首轮 ~0.6 s 于 ~2–5 倍负载）。
- **WM-0315 缺陷修复仍有效**：behavior 返回 `Wait` 时若有更多事件已入队会搁置卡死——50k 档在修复前无法跑完，修复后 5k/20k/50k 全绿（`batch.rs` `Wait` 分支与 `Continue` 一致）。
- 数字为单机单轮、共享负载下的快照；重复测量受抢占影响，绝对值和吞吐趋势在此负载下波动大（见 §2/§4）。

## 6. 相关

- WM-0320 soak：`crates/wm-runtime/tests/soak.rs`（`soak_short_repeated_cycles` CI 内跑；`soak_long_wallclock` 为 24h 目标，`#[ignore]` 按需跑，实测能力见 `docs/benchmarks/soak-capacity.md`）。

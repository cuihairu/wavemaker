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

测量时段 `/proc/loadavg`：`22.01 39.79 69.05`（14 CPU），即负载约为核数的 **1.6–5 倍**（较 WM-0011 测量时的 13–14 倍明显更轻，但仍非空载）。吞吐数字受 CPU 抢餐影响，**任何绝对数字都不可外推；只有相对趋势可引用**。

## 3. 吞吐结果（criterion 默认采样）

中位点估计（`thrpt` 区间中值）：

| 操作 | 5k Actor（20k timer） | 20k Actor（80k timer） | 50k Actor（200k timer） |
|---|---|---|---|
| lifecycle（端到端，每 actor 5 事件） | 652 Kelem/s（38.3 ms） | 500 Kelem/s（199.8 ms） | 410 Kelem/s（609.5 ms） |
| start（仅批量 start） | 721 Kelem/s（6.9 ms） | 351 Kelem/s（56.9 ms） | 235 Kelem/s（212.5 ms） |

## 4. 相对趋势（负载噪声下仍稳定成立）

1. **吞吐随规模下降**：lifecycle 从 5k→50k 降约 37%（652→410 Kelem/s），start 降约 67%（721→235 Kelem/s）。超线性下降来自批量调度下 actor 状态/队列的缓存效应与 runnable 队列峰值增长（20k/50k 时峰值 runnable 达整个 actor 数）。
2. **lifecycle 吞吐高于 start 的下降斜率**：lifecycle 的 per-actor 成本被 timer 触发与 resume 摊薄（每 actor 5 事件 vs start 的 1 事件），故规模化衰减更平缓。
3. **start 在 5k 档显著更快**（721 Kelem/s）：小规模下 runnable 队列贴合 L2/L3，批量 dequeue+run 命中率最高。

## 5. 已知边界与诚实标注

- **50k 全链路一次 ~0.6 s**（200k timer 事件）：本机在 ~2–5 倍负载下可稳定跑完 50k 操作点。
- **WM-0319 暴露并修复了 ActorRunner 的一个真实缺陷**：behavior 返回 `Wait` 时若已有更多事件入队（同 actor 多 timer 在 runnable 期间触发），事件会被搁置导致 actor 卡死。修复见 `fix: requeue actor on wait decision when events pending (wm-0315)`（`batch.rs`，`Wait` 分支现在与 `Continue` 一致：有 pending 事件则 requeue，否则 suspend）。**50k 档在修复前无法跑完**（虚拟超时），修复后 5k/20k/50k 全绿。
- 数字为单机单轮、共享负载下的快照；重复测量受抢占影响，趋势稳定而绝对值波动大。

## 6. 相关

- WM-0320 soak：`crates/wm-runtime/tests/soak.rs`（`soak_short_repeated_cycles` CI 内跑；`soak_long_wallclock` 为 24h 目标，`#[ignore]` 按需跑，实测能力见 `docs/benchmarks/soak-capacity.md`）。

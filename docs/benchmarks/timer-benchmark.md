# Timer Scheduler Benchmark（WM-0011，Phase 0 基线）

> 状态：Phase 0 基线（WM-0011 闭合材料；生产版 Timer Scheduler 属 Phase 3 WM-0304）
> 可执行验证：`cargo bench -p wm-core --bench timer_scheduler`
> 结论措辞约束：本机为多 agent 共享的高负载机器，以下数字**只用于相对比较**（规模之间、操作之间的趋势），不构成任何绝对性能承诺。

## 1. 被测对象

生产实现尚未落地（WM-0304，Phase 3）。本基线测的是其预定设计的最小代表实现（`crates/wm-core/benches/timer_scheduler.rs` 内嵌）：

- 数据结构：二叉最小堆（`BinaryHeap<Timer>`），按 `(deadline_ns, seq)` 排序；
- 取消：惰性墓碑（`HashSet<u64>`），pop 到堆顶时跳过——insert O(log n)、cancel 请求 O(1)、cancel 落地 O(log n) 摊销；
- 调度点规模：5k / 20k / 50k Actor × 每 Actor 4 timer（= 20k / 80k / 200k 个 timer）。

指标四组：`insert`（灌入全部 timer）、`cancel`（取消一半 + 全量 drain 触发墓碑路径）、`expire`（时钟推进到地平线，全量 pop-dispatch）、`fire-lateness`（真实时钟：按 2–36 ms 交错真实期限调度，自旋轮询派发，记录 `触发时刻 − 期限` 分布；轮询粒度 = 整批同一观察时刻，故 p50 偏保守）。

## 2. 负载上下文（重要）

测量时 `/proc/loadavg`：

| 时点 | 1min | 5min | 15min | 备注 |
|---|---|---|---|---|
| 第一轮开始 | 196.07 | 201.05 | 193.16 | 143–161 个可运行任务 / 14 CPU |
| 第二轮开始 | 172.56 | 193.52 | 193.89 | 同上 |

负载约为核数的 **13–14 倍**。吞吐数字受 CPU 抢餐影响显著偏保守且方差大（单组置信区间常达 ±60%）；lateness 受抢占支配，采样间抖动到数量级级别（50k 档个别样本在 500 ms 观察窗内只触发了一半 timer）。**任何绝对数字都不可外推；只有相对趋势可引用。**

## 3. 吞吐结果（第二轮，criterion `--measurement-time 2 --sample-size 20`）

中位点估计（`thrpt` 区间中值）：

| 操作 | 5k Actor（20k timer） | 20k Actor（80k timer） | 50k Actor（200k timer） |
|---|---|---|---|
| insert | 33.2 Melem/s | 17.8 Melem/s | 22.6 Melem/s |
| cancel（半量取消 + 全量 drain） | 557 Kelem/s | 254 Kelem/s | 231 Kelem/s |
| expire（全量 pop-dispatch） | 1.14 Melem/s | 652 Kelem/s | 1.59 Melem/s |

相对趋势（负载噪声下仍稳定成立）：

1. **insert 明显快于 cancel/expire**：堆 insert 是廉价路径；cancel 的代价主要在 drain 时逐条跳墓碑（摊销 O(log n) 而非 O(1)）；expire 的 pop 也是 O(log n)。
2. **cancel 吞吐随规模下降**（557K → 231K），与堆深度的对数增长一致。
3. **expire 吞吐在 20k 档最低、50k 档回升**——两轮测量中该非单调性都出现，判断为高负载下 CPU 迁移/抢占噪声所致，不据此下结构结论；待 WM-0304 生产实现与空闲机器复测。

## 4. 触发延迟（fire-lateness，12 个采样合并）

自旋轮询 + 惰性批派发的调度延迟分布（观察窗 = 最大期限 + 500 ms）：

| 规模 | 全部触发 | p50 中位 | p95 中位 | p99 中位 | 单样本最大 |
|---|---|---|---|---|---|
| 5k Actor | 12/12 样本全触发 | 2 ms | 19 ms | 26 ms | 50 ms |
| 20k Actor | 12/12 样本全触发 | 29 ms | 130 ms | 138 ms | 235 ms |
| 50k Actor | **3/12 样本有 timer 未在窗内触发**（最少 104k/200k） | 28 ms | 68 ms | 124 ms | 536 ms |

相对趋势（谨慎）：

1. 延迟随规模增长明显（5k 的 p99 ≈ 26 ms → 20k 的 ≈ 138 ms），符合「到期批量越大、单批派发时间越长」的预期。
2. 50k 档出现**窗口内触发不全**的样本：负载 ~13× 核数时，进程被抢占数百 ms 是现实。这佐证计划书 R-002（callback 精度不足 → 到达计划偏差）的方向：**生产 Worker 必须声明调度分辨率并按数据质量标签降级（ObserverDegraded/GeneratorSaturated 路径），而不是假设 timer 准点**。
3. 本测量用的自旋轮询是刻意的基线上界（烧满一个核换取最小轮询粒度）；生产实现大概率用 condvar/epoll 等待，粒度特征不同。本组数字**只说明「堆调度本身不是瓶颈，抢占才是」**。

## 5. 复现与遗留

```bash
# 全量四组（本机高负载下约 2–4 分钟）
cargo bench -p wm-core --bench timer_scheduler

# 快速复测（同本文参数）
cargo bench -p wm-core --bench timer_scheduler -- --measurement-time 2 --warm-up-time 1 --sample-size 20
```

遗留（转入既有任务）：

- 生产版 Timer Scheduler 及其 criterion 接入：WM-0304（Phase 3）；届时本文件的数字作废，仅保留方法学。
- 空闲/独占机器上的复测与 5k/20k/50k 全链路基准：WM-0319。
- KBEngine callback 调度粒度的实测对照：WM-0010（静态侧见 docs/spec/kbengine-api-audit.md）。

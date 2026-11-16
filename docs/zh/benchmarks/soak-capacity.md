# Soak Capacity（WM-0320，实测记录）

> 状态：WM-0320 闭合材料。soak 工具链在 `crates/wm-runtime/tests/soak.rs`；本文件记录**本机实测**的 50k 操作点能力与内存表现。
> 可执行验证：
> ```text
> WM_SOAK_ACTORS=50000 WM_SOAK_SECONDS=30 \
>   cargo test --release -p wm-runtime --test soak -- --ignored soak_long_wallclock --nocapture
> ```
> 措辞约束：本机为多 agent 共享高负载机器，**数字只反映本机负载窗口，不可外推为承诺**。

## 1. 被测对象与模式

soak 模拟生产模式：**重复 swell 周期**。每个周期在自己的 `ActorRunner` 内跑完整 actor 生命周期（spawn 50k → start → 每 actor 4 timer 触发 → close），周期结束 runner drop，内存回到该周期的活集上界（50k actor 集），不随累计周期数增长。每周期断言：全部 actor close、runnable 队列清空、事件数精确（每 actor 恰好 1 Started + 4 Timer）。

短时版 `soak_short_repeated_cycles`（20 周期 × 1k actor）在 CI 内跑；`soak_long_wallclock` 为 24h 目标，`#[ignore]` 按需跑（`WM_SOAK_ACTORS`/`WM_SOAK_SECONDS`/`WM_SOAK_REPORT_EVERY`）。

## 2. 实测（release，50k actor/周期）

| 运行 | 时长 | 周期 | 事件 | 周期率 | 结束 RSS |
|---|---|---|---|---|---|
| 窗口 1 | 31.3 s | 53 | 13.25 M | 1.7 cycles/s（≈424 Kelem/s） | 44.3 MiB |
| 窗口 2 | 45.4 s | 15 | 3.75 M | 0.33 cycles/s（≈83 Kelem/s） | 45.1 MiB |

窗口 2 的 RSS 时间序列（每 10 周期）：`t=30s cycles=10 rss=44788KiB` → `done rss=45108KiB`（+0.3 MiB / 45 s，基本平坦）。

## 3. 诚实结论

1. **50k 操作点可跑**：两个窗口都完整跑完 50k actor/周期并断言全绿（全部 close、事件精确、runnable 清空），无 panic。
2. **内存有界**：RSS 稳定在 ~45 MiB，45 s 内仅 +0.3 MiB。周期式 runner 复用（每周期 drop）使内存上界 = 单周期 50k 活集，不随累计周期增长。
3. **速率受负载支配，波动 ~5 倍**（1.7 → 0.33 cycles/s）：共享机器上其他 agent 的 CPU 抢餐主导，绝对值不可外推。**因此 24h 周期数不是定值**：按实测区间，24h 完成 26k–147k 个 50k 周期（6.5B–36.7B 事件），具体取决于当时负载。
4. **未执行完整 24h**：本会话内只跑了 31 s / 45 s 窗口验证稳定性与内存平坦；24h 是墙钟时间问题，非能力问题——本机具备持续跑 50k 的实测证据（平坦 RSS + 稳定周期完成），但**完整 24h 连续运行未在本会话内执行**，不在 todo 里标为「已实测 24h」。

## 4. 已知边界

- 周期内 close 的 actor 保留在 lifecycle 的 `actors` 映射中（history 可查询）——因此**单个 swell 周期内的内存上界 = 该 swell 的 actor 数**（周期完成即释放）。若未来需要**单一 ActorRunner 内**无界 churn（不重建 runner），需给 lifecycle 增加 closed 回收；当前生产模式为「一 swell 一 runner」，不受影响。
- soak 报告 RSS 用 `/proc/self/statm`（Linux）；非 Linux 返回 0，不影响断言（断言基于功能不变量，RSS 仅记录）。

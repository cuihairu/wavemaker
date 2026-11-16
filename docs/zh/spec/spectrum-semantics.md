# Spectrum Semantics: Arrival / Lifetime / Concurrency（BLOCKER-1 闭合）

> 状态：Phase 0 草案（WM-0003 / WM-0004 / WM-0005）
> 来源：计划书 §8.1 / §8.2 / §8.2.1 / §8.2.2。本文只写计划书已有内容；推断处标注「建议」或「待验证」。
> 可执行验证：`tools/spectrum_sim.py` + `tools/test_spectrum_sim.py`（`python3 -m pytest tools/test_spectrum_sim.py`，14 passed）。
> 注：计划书引用的 `wavemaker-stack-decision.md` 不在本地，涉及原始 ADR 结论处标注「待原始决策文档确认」。

## 1. 问题（计划书 §8.2）

原始 ADR 示例（待原始决策文档确认）：

- arrival 从 1/s 指数增长到 20/s；
- ramp 600 s；
- lifetime 均值 900 s；
- concurrency 目标 5000。

核心矛盾：**单凭 lifetime 均值不能推出精确并发曲线**。指数到达在 ramp 期积分约 3800（本文解析值 3805.4）；若 lifetime 固定 900 s，这些 Bot 在 t=600 s 时全部未到期；若 lifetime 是指数分布（均值 900 s），t=600 s 时活跃量明显更低。Little's Law `L = λW` 不能不加条件地用于短时 ramp。

## 2. 定义（建议的 v1 语义，计划书 §8.2.1）

Spectrum 必须分开表达以下量，禁止混成一个含糊的 `users` 字段（计划书 §8.1）：

| 字段 | 语义 | 数学符号 |
|---|---|---|
| `arrival.target_rate` | 目标到达速率；支持 ramp 模型（`from_per_second` → `to_per_second` over `duration`） | λ(t) |
| `concurrency.target` | 希望达到的并发目标（可选） | — |
| `concurrency.max` | 硬性并发上限（可选） | C_max |
| `lifetime.distribution` | 生命周期概率分布（`fixed` / `exponential` 等） | L ~ F(·) |
| `saturation_policy` | 达到 `concurrency.max` 后的处理方式 | — |

派生量：

- 累计到达 Λ(t) = ∫₀ᵗ λ(s) ds；
- 活跃量（并发）N(t) = |{ i : t_i ≤ t < t_i + L_i }|，即已创建且未到期的 Bot 数。

## 3. 指数到达积分

ramp 模型取指数增长：λ(t) = λ₀·e^{kt}，k = ln(λ₁/λ₀)/T_ramp，ramp 结束后保持 λ₁（建议；计划书示例未显式说明 ramp 后行为，待确认）。

ramp 期创建总量：

```text
Λ(T_ramp) = (λ₁ − λ₀) / k = 11400 / ln(20) ≈ 3805.4
```

与计划书「约 3800」一致。

## 4. 活跃量曲线推导

### 4.1 一般式

```text
N(t) = ∫₀ᵗ λ(s) · P(L > t − s) ds = ∫₀ᵗ λ(s) · S̄(t − s) ds
```

其中 S̄ 为 lifetime 的生存函数。

### 4.2 固定 lifetime（L = 900 s）

```text
N(t) = Λ(t) − Λ(t − 900)
```

t = 600 s 时：N(600) = Λ(600) − Λ(−300) = 3805（ramp 期创建的 Bot 全部未到期）。

### 4.3 指数 lifetime（均值 μ = 900 s）

```text
N(t) = ∫₀ᵗ λ(s) · e^{−(t−s)/μ} ds
```

t = 600 s 时数值积分 ≈ 3192，明显低于固定 lifetime 的 3805。**这证明 BLOCKER-1 的结论：必须显式声明 `lifetime.distribution`，只有均值不够。**

### 4.4 Little's Law 适用边界

`L = λW` 仅在稳态成立：λ 恒定且 t ≫ W（暂态消亡后）。本文模拟中，hold 相位维持 20/s 达 3000 s（≫ 900 s）后，N(3600) ≈ 17 615–17 861（模拟）≈ 18 000（λ·W，误差 < 3%，Poisson 噪声范围内）。而 t = 600 s 时 naive λ(t)·W = 18 000，是真实活跃量（约 3300–3805）的 4.7–5.5 倍。

**校验规则（建议）**：`concurrency.target` 只能作为（按 lifetime 分布模拟或解析估算得到的）活跃量预测的对照值，不得用 λ(t)·W 直接换算。

## 5. Saturation Policy 语义（计划书 §8.2.1，WM-0004）

达到 `concurrency.max` 后：

| policy | 语义 | 必须记录 | 模拟行为（cap=5000） |
|---|---|---|---|
| `reject_new` | 不再创建新 Bot | rejected arrivals 计数 | admitted 20 000，rejected 43 836 |
| `queue` | 排队，按 FIFO 在容量释放时创建 | 实际到达时间偏离计划时间的偏差（actual_at − planned_at） | admitted 20 000，mean delay 939.4 s，p99 1960.9 s |
| `delay` | 延后到容量可用时创建（planned_at 平移，不保证顺序） | 调度偏差 | 同 queue 的数值行为，语义差异在 planned_at 平移与顺序保证 |

结论约束：`queue` 与 `delay` 都不丢负载但引入到达偏差，报告必须呈现偏差分布；`reject_new` 丢负载，报告必须呈现 rejected 计数。三者均不得静默：校验输出必须可解释（计划书 §8.2.2）。

## 6. 校验规则（计划书 §8.2.2）

1. `arrival` 与 `concurrency.target` 同时存在时，**不应无条件拒绝**；
2. 根据 lifetime 分布**模拟或解析估算活跃量曲线**（本文 §7 的脚本即该估算器）；
3. 若 `target` 与预测值差异超过阈值，发出配置警告或要求用户选择优先级；
4. 若 `concurrency.max` 小于计划负载的必要容量，明确拒绝或按 `saturation_policy` 执行；
5. 所有校验输出可解释的数值和假设。

阈值取值（建议，待验证）：相对偏差 > 10% 时告警。此阈值为工程建议，计划书未给出数值。

## 7. 可执行验证（WM-0005）

脚本 `tools/spectrum_sim.py`：时间步长 dt=0.5 s 的 Poisson 模拟（seed=20261104，可复现），对照解析式。运行：

```bash
python3 tools/spectrum_sim.py                      # 输出结果表
python3 -m pytest tools/test_spectrum_sim.py       # 14 个单测
```

单测覆盖：指数到达积分 ≈3800；固定/指数 lifetime 模拟与解析式一致（5% 容差）；t=600 s 时固定 lifetime 全部未到期、指数 lifetime 更低；稳态 Little's Law 成立（t=3600）、ramp 期不成立；三种 saturation policy 的 cap 约束与偏差记录；同 seed 确定性。

### 模拟结果（实际运行输出，2026-11-05）

#### 活跃量 N(t)：模拟 vs 解析参照

场景：λ(t) 1/s → 20/s 指数增长 600 s 后保持；lifetime 均值 900 s；seed 20261104；dt 0.5 s；horizon 3600 s。Ramp 总创建量 Λ(ramp) = 3805.4。

| t (s) | Λ(t) (analytic) | N(t) fixed (sim) | N(t) fixed (analytic) | N(t) exponential (sim) | N(t) exponential (analytic) | λ(t)·W (Little) |
|---:|---:|---:|---:|---:|---:|---:|
| 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| 100 | 130 | 123 | 130 | 120 | 123 | 1483 |
| 200 | 343 | 329 | 343 | 325 | 313 | 2443 |
| 300 | 695 | 684 | 695 | 638 | 615 | 4025 |
| 400 | 1275 | 1228 | 1275 | 1094 | 1102 | 6631 |
| 500 | 2231 | 2152 | 2231 | 1954 | 1895 | 10925 |
| 600 | 3805 | 3791 | 3805 | 3283 | 3192 | 18000 |
| 900 | 9805 | 9852 | 9805 | 7509 | 7390 | 18000 |
| 1200 | 15805 | 15282 | 15110 | 10321 | 10398 | 18000 |
| 1500 | 21805 | 18234 | 18000 | 12500 | 12553 | 18000 |
| 1800 | 27805 | 18072 | 18000 | 14141 | 14097 | 18000 |
| 2400 | 39805 | 17901 | 18000 | 15860 | 15996 | 18000 |
| 3000 | 51805 | 17850 | 18000 | 17006 | 16971 | 18000 |
| 3600 | 63805 | 17860 | 18000 | 17615 | 17472 | 18000 |

注：t=1200–3600 s 时 N(t) fixed (sim) 与 analytic 的差异来自 Poisson 到达噪声（分析值假设到达确定性分布）；误差 < 5%，在统计容差内（见单测）。

#### Saturation policies（concurrency.max = 5000）

| policy | admitted | rejected | max active | mean delay (s) | p99 delay (s) |
|---|---:|---:|---:|---:|---:|
| reject_new | 20000 | 43836 | 5000 | 0.0 | 0.0 |
| queue | 20000 | 0 | 5000 | 939.4 | 1960.9 |
| delay | 20000 | 0 | 5000 | 939.4 | 1960.9 |

## 8. BLOCKER-1 闭合结论

1. `arrival`、`lifetime.distribution`、`concurrency.target/max`、`saturation_policy` 是四个独立字段，缺一不可（计划书 §8.2.1，本文 §2）。
2. 单个 lifetime 均值不足以预测并发曲线：固定 vs 指数分布在 t=600 s 相差约 600+ 活跃 Bot，且随时间扩大（t=1500 s 时相差约 5 700）。
3. `concurrency.target` 是对照值不是设定值；必须经活跃量预测（模拟/解析）校验（§6）。
4. naive Little's Law 仅限稳态参考（§4.4）。

待办遗留：ramp 结束后 arrival 是否保持 λ₁（本文按保持模拟）——计划书未明示，待原始决策文档确认；threshold 数值与解析估算器接入 Spectrum 校验器的形式，在 Phase 1 `wm validate` 实现时冻结。

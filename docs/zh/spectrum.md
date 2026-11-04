---
title: Spectrum 与确定性计划
---

# Spectrum 与确定性计划

## 8.1 Spectrum 结构

Spectrum 应分开表达：

- Arrival Model：Bot 到达过程。
- Concurrency Constraint：并发目标或上限。
- Lifetime Model：Bot 生命周期分布。
- Behavior Model：行为组合。
- Workload Model：请求/消息和业务负载特征。
- Fault Model：故障注入策略（如启用）。

不要把到达速率、并发数量和 Bot 生命周期混成一个含糊的 `users` 字段。

## 8.2 arrival / concurrency 阻塞项

当前 ADR 中存在以下示例：

- arrival 从 1/s 指数增长到 20/s。
- ramp 600s。
- lifetime 均值 900s。
- concurrency 目标 5000。

指数到达积分约 3800，表示 600 秒内的创建总量；在固定 900 秒生命周期假设下，这些 Bot 在 600 秒时仍未到期。若 lifetime 是指数分布，则 600 秒时活跃量会更低。

因此，单凭 lifetime 均值不能推出精确并发曲线。长期稳态下可参考 Little's Law：

\[
L = \lambda W
\]

但不能不加条件地将其用于短时 ramp。

### 8.2.1 建议的 v1 语义

明确区分：

- `arrival.target_rate`：目标到达速率。
- `concurrency.target`：希望达到的并发目标（可选）。
- `concurrency.max`：硬性并发上限（可选）。
- `lifetime.distribution`：生命周期概率分布。
- `saturation_policy`：达到 max 后的处理方式。

建议 saturation policy：

- `reject_new`：不再创建，计入 rejected arrivals。
- `queue`：排队，实际到达时间偏离计划时间，必须记录。
- `delay`：延后到容量可用时创建，记录调度偏差。

### 8.2.2 校验规则

- arrival 与 target concurrency 同时存在时，不应无条件拒绝。
- 根据 lifetime 分布模拟或解析估算活跃量曲线。
- 若 target 与预测值差异超过阈值，发出配置警告或要求用户选择优先级。
- 若 `max_concurrency` 小于计划负载的必要容量，明确拒绝或按 saturation policy 执行。
- 所有校验输出可解释的数值和假设。

**BLOCKER-1 必须在 Phase 0 闭合，并形成可执行测试。**

## 8.3 Plan-as-data

Controller 是唯一计划生成者。

计划数据至少包含：

```text
PlanEntry
  actor_or_bot_id
  backend_id
  shard_id
  planned_at
  lifetime
  behavior_group
  seed
  attributes
```

Engine Bot Backend 如果无法消费逐 Bot 计划，应由 Controller 将计划转换为其可表达的批次计划，同时保存转换前后的信息。

禁止 Worker 或 Bridge 根据 `(seed, lambda, W, i)` 重新计算全局到达计划。

## 8.4 ShardPlan 冻结

- Swell 创建时冻结 Worker 数量 W 和 ShardPlan。
- 计划确定后记录 plan_digest。
- 执行中不得因 Worker 临时数量变化而静默改变分片规则。
- 故障恢复应有明确的重分配和去重策略。
- 到达序列的分片必须通过全局 conformance test 验证。
- ADR 原有时间变换法 `i mod W` 方案需保留为待验证决策，不应在未测试前视为已证明最优。

## 8.5 确定性范围

确定性分为：

1. Plan Determinism：同输入得到相同计划。
2. Runtime Determinism：给定计划和事件序列，运行时状态变化可重放。
3. Behavior Determinism：行为代码在相同输入下产生相同动作。

跨语言插件不能默认保证第三层。每个插件必须声明确定性等级和外部副作用。

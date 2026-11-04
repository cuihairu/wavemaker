# ADR-006: Plan-as-Data and ShardPlan Freezing

> 状态：Accepted
> 来源：计划书 §0.1.6 / §8.3 / §8.4 / §8.5 / §23。WM-0006 闭合材料。

## Context（背景）

跨进程契约使用 Protobuf IDL，禁止跨层共享 ABI（计划书 §0.1.6）。Controller 统一生成计划，Backend 只消费计划数据。若允许 Worker/Bridge 端依据 `(seed, lambda, W, i)` 自行重算全局到达计划，计划将无法确定、无法审计、无法对比「计划负载 vs 实际负载」。

## Decision（决策）

### D1：Controller 是唯一计划生成者（计划书 §8.3）

- 计划以数据形式（PlanEntry 序列）生成、传输、存储与校验；Backend 只消费，不重算。
- **禁止** Worker 或 Bridge 根据 `(seed, lambda, W, i)` 重新计算全局到达计划（§8.3 原文）。
- Engine Bot Backend 若无法消费逐 Bot 计划，由 Controller 将计划**转换为其可表达的批次计划，同时保存转换前后的信息**（§8.3）——转换是显式、可审计的，不是静默降级。

### D2：PlanEntry 字段（计划书 §8.3）

| 字段 | 含义 |
|---|---|
| `actor_or_bot_id` | 目标 Actor（Native）或 Bot（Engine Bot Backend）标识 |
| `backend_id` | 承载该条目的 Backend |
| `shard_id` | 所属分片 |
| `planned_at` | 计划到达/创建时间 |
| `lifetime` | 该 Bot 的生命周期（按 Spectrum 的 lifetime.distribution 生成的确定值） |
| `behavior_group` | 行为组引用 |
| `seed` | 该条目的确定性随机源种子 |
| `attributes` | 扩展属性 |

字段类型与 Protobuf 消息定义在 Phase 1 冻结（WM-0102），本 ADR 只冻结字段集合与语义。

### D3：ShardPlan 冻结规则（计划书 §8.4）

1. Swell 创建时冻结 Worker 数量 W 和 ShardPlan。
2. 计划确定后记录 `plan_digest`；Backend 接收时校验摘要一致（计划书 §16.3）。
3. 执行中不得因 Worker 临时数量变化而静默改变分片规则。
4. 故障恢复必须有明确的重分配和去重策略。
5. 到达序列的分片必须通过全局 conformance test 验证。
6. 原始 ADR 的时间变换法 `i mod W` 方案**保留为待验证决策**，不在未测试前视为已证明最优（§8.4 原文；待原始决策文档确认其完整定义）。

### D4：确定性分层（计划书 §8.5）

1. **Plan Determinism**：同输入得到相同计划（本 ADR 保证）。
2. **Runtime Determinism**：给定计划和事件序列，运行时状态变化可重放。
3. **Behavior Determinism**：行为代码在相同输入下产生相同动作；跨语言插件不能默认保证此层，每个插件必须声明确定性等级和外部副作用。

## Consequences（后果）

- 「计划负载 vs 实际到达」可逐条对比，支撑报告第 3 节（Planned vs Actual Arrival）与 Test Validity 判定（计划书 §13.2 / §16.3）。
- 计划转换（逐 Bot → 批次）必须保存前后信息，报告可展示转换造成的精度损失（如 Engine Bot 的 batch_arrival 调度分辨率）。
- 分片规则变更、重分配、去重均需 conformance test 覆盖；不能靠「接口已预留」代替可运行测试（计划书 §21）。
- PlanEntry 的 Protobuf 消息初稿属于 Phase 1 WM-0102；`i mod W` 的验证属于 Phase 5 分布式分片实现（WM-0504）。

# ADR-007: Assertion Provider——业务断言外置与调度

> 状态：Accepted
> 来源：用户需求（新增能力方向）+ 设计文档 docs/spec/assertion-provider.md。报告扩展落点为 schemas/report/v1.json 第 15 节与 todo.md Phase 7。

## Context（背景）

负载与观测指标回答「系统扛住了多少」，回答不了「系统做对了没有」：1 万人登录成功不代表没有重复创建角色，5 千人匹配成功不代表没有重复房间，服务端稳定不代表货币/背包一致，故障恢复成功不代表战斗奖励没有重复结算。业务语义正确性只有懂业务的一方（游戏服务端查询接口、StateDiff 工具、对账脚本）能判定。

Wavemaker 是压测框架：若把业务规则内置进核心，框架将被单一游戏语义绑架，且每条规则都变成无法测试的黑盒预言机。

## Decision（决策）

### D1：Wavemaker 不理解业务语义，不做预言机

- 核心代码不出现任何游戏业务概念，不内置任何业务断言规则。
- Wavemaker 只做五件事：**注册**（断言清单随 Swell 冻结进 ShardPlan，遵守 ADR-006 plan-as-data）、**调度**（给 Provider 执行窗口）、**登记**（结果进报告）、**联动**（结果影响数据质量与 Test Validity）、**归档**（证据落盘）。
- 任何「内置一条业务断言」的提议在评审中应被驳回。

### D2：业务语义由外部 Assertion Provider 提供

- Provider 是进程边界外的独立实现（游戏 GM/管理 API、只读从库查询、StateDiff 工具、日志稽核器等），单一契约动作 `Execute(name, params, window_context) -> AssertionResult`，协议实现期冻结（倾向复用 Protobuf 约束）。
- 结果三态：`pass | fail | inconclusive`，缺一不可；Provider 不返回一律记 `inconclusive`，**禁止**把「没跑」渲染成「通过」。

### D3：三个执行窗口，全部挂在既有生命周期钩子上

1. `after_scenario`：场景结束评估（窗口 = 场景实际起止）。
2. `after_recovery`：故障恢复握手与状态同步完成后（Phase 5 WM-0509 钩子；这是「恢复成功 ≠ 奖励不重复结算」的落点）。
3. `periodic`：周期探针，只记录、不参与瞬时熔断。

断言执行不阻塞到达调度；Swell 收尾前未返回的结果记 `inconclusive`。

### D4：报告与有效性联动

- 报告新增**可选**第 15 节 `assertions`（可选保证既有报告向后兼容）；数据源标签新增 `AssertionProvider`，与 Basin 区分以保护 §13.3 R2 语义。
- Schema 约束 R5：任一断言 `fail` 时 `test_validity.verdict` 必须为 `Invalid` 或 `Partial`——业务违例直接否定测试有效性。
- 断言失败**不自动终止 Swell**：是否熔断由既有 StopAdmission 机制（WM-0507）按 `on_fail: mark_critical` 配置决定，不新造终止路径。

### D5：证据保存

- Provider 证据原样落盘 `<swell_dir>/evidence/<assertion-name>/<timestamp>/`（Controller 预创建并注入路径，只归档不解释）；引用进报告 `raw_data_references`（新格式 `evidence-bundle`）。
- 原始证据不转换、不采样、不汇总——框架没有能力判断业务证据中什么是冗余的。保存失败必须显式降级标注，不得静默丢弃引用。

## Consequences（后果）

- 断言结果的质量与 Provider 实现强相关：Provider 挂了只能得到 `inconclusive`，这会被如实记录为覆盖缺口而不是被掩盖。
- Schema、报告模板、双语文档站、todo.md（Phase 7，WM-0701 起）需同步；本 ADR 与 docs/spec/assertion-provider.md 为其唯一规范来源。
- Schema 扩展已随本 ADR 落地（24 项单测覆盖 R5 与三态语义）；Provider 协议、执行窗口调度、证据归档属于 Phase 7 实现（WM-0702 起），未实现前不得在报告里手写 `assertions` 节以外的断言结论。

# Assertion Provider：业务正确性断言设计（v0 草案）

> 状态：Phase 0 设计草案（新增能力方向，尚未绑定既有 WM 任务编号；见 todo.md Phase 7）
> 来源：用户需求 + ADR-007。与计划书 §13.2 报告结构、§11.3 数据质量标签、§16.3 测试有效性衔接。
> 相关文件：`schemas/report/v1.json`（`assertions` 节）、`docs/adr/ADR-007-assertion-provider.md`、`tools/validate_report.py`。

## 1. 动机

负载指标回答「系统扛住了多少」，回答不了「系统做对了没有」：

- 1 万人登录成功 ≠ 没有重复创建角色；
- 5 千人匹配成功 ≠ 没有重复房间；
- 服务端稳定 ≠ 货币/背包数据一致；
- 故障恢复成功 ≠ 战斗奖励没有重复结算。

这类「业务语义正确性」只有游戏服务端自己（或懂它的适配器）能判定。Wavemaker 是压测框架，不是业务预言机：它不应内置任何游戏规则，但必须为懂规则的外部检查留出正式的接入位、结果通道和证据链。

## 2. 边界（不可协商）

1. **Wavemaker 不理解业务语义，不做预言机。** 核心代码不出现「角色」「背包」「房间」等概念，也不内置任何一条业务断言规则。
2. 业务语义由**外部 Assertion Provider** 提供：游戏服务端查询接口（如 GM/管理 API、只读从库）、状态比对工具（StateDiff）、对账脚本、日志稽核器等。Provider 是独立进程/服务，崩溃、超时、误报都不应拖垮压测执行本身。
3. Wavemaker 只负责三件事：**调度**（在正确的时机给 Provider 执行窗口）、**登记**（结果进入报告）、**联动**（结果影响数据质量标签与 Test Validity）。
4. Provider 结果是**证据，不是裁决**：`fail` 只强制报告降级（见 §5），不自动终止 Swell；是否终止由 Swell 配置的熔断策略决定（Phase 5 WM-0507 的 StopAdmission 机制复用，不新造）。

## 3. Provider 接口草案

Provider 以声明式清单注册（配置文件，随 Swell 冻结进 ShardPlan，遵守 ADR-006 plan-as-data）：

```yaml
# swell 配置中的 assertions 段（示意，字段在实现前仍可调整）
assertions:
  - provider: game-admin-api        # Provider 实现标识
    name: no-duplicate-roles        # 断言实例名，报告内唯一
    when:                           # 执行窗口，见 §4
      - after_scenario: create-role
      - after_recovery: true
      - periodic:
          interval_s: 60
    timeout_s: 30
    on_fail: record                 # record（默认）| mark_critical（参与熔断）
    params:                         # 透传给 Provider 的业务参数
      endpoint: http://gm.internal/audit/duplicate-roles
      window: swell
```

### 3.1 Provider 与 Wavemaker 的契约

Wavemaker 与 Provider 之间是**进程边界**（HTTP/gRPC/命令行均可，实现期冻结协议；倾向复用 Protobuf 约束，见 docs/spec/protobuf-conventions.md）。交互只有一种动作：

```
Execute(name, params, window_context) -> AssertionResult
```

`window_context` 携带 swell_id、backend_id、时间窗口 `[window_start, window_end]`、触发原因（scenario 结束 / 恢复完成 / 周期探针），使 Provider 能圈定检查范围。

### 3.2 结果结构

```json
{
  "provider": "game-admin-api",
  "name": "no-duplicate-roles",
  "status": "pass | fail | inconclusive",
  "checked_at": "<RFC3339>",
  "window": {"start": "<RFC3339>", "end": "<RFC3339>"},
  "message": "3 duplicate role pairs found for accounts 1042,1087,2210",
  "evidence": [
    {"kind": "artifact", "ref": "swell-001/evidence/no-duplicate-roles-diff.json"},
    {"kind": "query", "ref": "SELECT count(*) FROM roles GROUP BY account HAVING count(*)>1"}
  ]
}
```

状态只有三种，且**三态缺一不可**：

| 状态 | 语义 | 报告联动 |
|---|---|---|
| `pass` | Provider 在窗口内完成检查且未发现违例 | 如实记录 |
| `fail` | Provider 完成检查并发现违例 | Test Validity 强制降级（§5） |
| `inconclusive` | Provider 未能完成检查（超时、接口不可用、覆盖不全） | 按数据质量 `Missing`/`ObserverDegraded` 路径处理，不伪装成 pass |

禁止第四种隐式状态：Provider 不返回 = `inconclusive`，绝不允许「没跑」被渲染成「通过」。

### 3.3 Provider 自身的数据质量

Provider 的检查结果也要接受 §11.3 式的审视：Provider 挂了、返回覆盖不全、时间窗对不上时，对应条目标 `inconclusive`，其 quality 按 `Missing`/`Delayed` 标注。断言通道本身的开销计入观测开销预算（§10.6，observer_baseline），由 WM-0411 的降级机制统一管理。

## 4. 与 Swell 生命周期的集成点

三个执行窗口，全部落在既有的生命周期钩子上，不新增生命周期阶段：

1. **场景后评估（after_scenario）**：某个 behavior_group / scenario 结束时触发。用于「这批 Bot 做完这件事之后，业务状态是否正确」。窗口 = 该场景的实际起止时间。
2. **故障恢复后断言（after_recovery）**：Worker/Backend 恢复握手完成、状态同步结束后触发（Phase 5 WM-0509 的钩子）。这是「故障恢复成功 ≠ 奖励没有重复结算」的落点：恢复指标全绿之后，业务对账才真正回答「恢复得对不对」。窗口 = 故障注入开始到恢复完成的区间。
3. **周期性探针（periodic）**：按固定间隔在执行期采样。用于捕捉「何时开始不一致」，为瓶颈分析提供时间相关性输入。探针只记录，不参与瞬时熔断。

调度原则：

- 断言执行窗口不打进关键路径：Provider 在窗口触发时被调用，但不阻塞 Generator 的到达调度；
- `after_scenario` / `after_recovery` 的结果必须在 Swell 收尾（报告生成前）全部收齐；逾期未返回按 `inconclusive` 记录；
- 周期探针的结果按时间戳对齐进报告（遵守 §10.4 三时间戳约定）。

## 5. 报告集成

### 5.1 新增报告节

在计划书 §13.2 的 14 节结构上**扩展**（不重排既有 14 节）第 15 节：

| # | Section（schema key） | 核心数据字段 | 数据源 |
|---|---|---|---|
| 15 | `assertions` | results[]: provider, name, status(pass/fail/inconclusive), message, evidence[] | Provider（新增数据源标签，见 §5.2） |

Schema 落点：`schemas/report/v1.json` 新增可选 `sections.assertions`（可选保证既有报告向后兼容）；约束 **R5**：任一断言 `status == "fail"` 时，`test_validity.verdict` 必须是 `Invalid` 或 `Partial`——业务语义违例直接否定测试有效性，这与 §16.3 的既有判定条件是叠加关系（fail_criteria 增列 `assertion:<name>:fail`）。

### 5.2 数据源标签

`data_source` 枚举新增 `AssertionProvider`：断言结果不是 Planned（计划）、不是 Actual（发生器）、也不是 Basin（常规服务端观测），混用标签会破坏 §13.3 的 R2 约束语义。携带 `AssertionProvider` 的 section 的 quality 遵守 §11.3 同一套标签。

### 5.3 数据质量联动汇总

| Provider 状态 | section quality | test_validity.verdict | confidence |
|---|---|---|---|
| 全部 pass | Complete（若窗口完整） | 不受断言影响 | 不受断言影响 |
| 任一 fail | Complete 或按实际覆盖 | **≥ Partial**（R5 强制，通常 Invalid） | 建议降低（约束层暂不强制 confidence，避免与 R3 语义冲突） |
| 任一 inconclusive | Partial / Missing / ObserverDegraded（按原因） | 由既有 §16.3 覆盖规则判定 | 经 R3 若覆盖不足则强制 low |

## 6. 失败时的证据保存

`fail` 的价值取决于证据能否复盘。约定：

1. Provider 返回的 `evidence[]` 原样写入报告的 `assertions` 节，`kind: artifact` 的引用登记进第 14 节 `raw_data_references`（format 按实际，新增 `evidence-bundle`）。
2. 证据文件落盘路径约定：`<swell_dir>/evidence/<assertion-name>/<timestamp>/`，由 Controller 在登记断言实例时预创建并注入 `window_context`，Provider 直接写入；Controller 只归档，不解释内容。
3. 证据保存是**尽力而为但必须可见**：保存失败时，`evidence` 保留引用并标记 quality 降级，报告不得静默丢弃失败断言的证据引用。
4. 原始证据不做格式转换、不采样、不汇总——Wavemaker 没有能力判断业务证据里什么是冗余的。

## 7. 实现任务与边界回顾

实现分解见 todo.md Phase 7（WM-0701 起）。重申边界：Wavemaker 交付的是**注册、执行窗口、结果登记、报告联动、证据归档**这五件调度层的事；每一条具体的「什么算正确业务状态」都属于外部 Provider。评审本设计时，任何「内置一条业务断言」的提议都应被驳回。

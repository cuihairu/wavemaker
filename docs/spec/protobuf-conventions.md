# Protobuf Conventions（WM-0013）

> 状态：Phase 0 草案（包名与兼容策略冻结候选；错误码为骨架，Phase 1 WM-0103 冻结）
> 来源：计划书 §0.1.6 / §3.1 / §5.2 / §5.3 / §12 / §17 / §23。

## 1. 包名约定

所有跨进程契约使用 Protobuf IDL（计划书 §0.1.6：禁止跨层共享 ABI）。包名统一为：

```text
wavemaker.common.v1     # 通用类型：OperationResult、错误码、时间戳、generation/epoch、分页/cursor
wavemaker.control.v1    # Controller ↔ Worker/Supervisor 控制面：注册、心跳、租约、Swell 管理
wavemaker.backend.v1    # Execution Backend 接口（计划书 §4.1 的领域操作：handshake/apply_plan/...）
wavemaker.engine.v1     # 引擎 Bridge 协议（计划书 §5.3 草案已使用 wavemaker.engine.v1）
wavemaker.metrics.v1    # 指标与事件上报：Generator 指标、HDR V2 blob、事件流
```

与计划书 §17 目录结构（`proto/{common,control,backend,engine,metrics}/v1/`）一一对应。

文件命名：`<domain>_<message_group>.proto`（snake_case），每个文件显式声明：

```protobuf
syntax = "proto3";
package wavemaker.control.v1;
```

（是否迁移到 Protobuf Edition 待 Phase 1 codegen 验证后决定，WM-0102；不提前写死。）

消息与字段命名：消息 PascalCase，字段 snake_case，枚举值 SCREAMING_SNAKE_CASE；枚举必须带 `*_UNSPECIFIED = 0` 之外的显式零值语义（建议：零值保留为 UNSPECIFIED，业务状态从 1 开始）。

## 2. 版本兼容策略

1. **主版本入包名**：破坏性变更产生新包 `wavemaker.*.v2`，不修改既有 v1 语义。
2. **v1 内只允许安全演进**（proto3 规则）：仅新增字段/枚举值/服务方法；不得修改既有字段号；废弃字段必须 `reserved` 字段号与名称，不得复用。
3. **握手拒绝不静默降级**（计划书 §5.3.1）：Handshake 携带 `protocol_version`；协议版本不兼容时必须拒绝启动。Worker/Bridge 注册信息含 protocol_version、adapter version、engine/runtime version（计划书 §12.1）。
4. **generation/epoch 防陈旧状态**（计划书 §4.1.1/§7.3/§12.2）：所有状态事件携带 generation；旧 generation 的迟到事件丢弃或归档，不得覆盖新状态。
5. **跨语言一致性**：不同语言 Protobuf codegen 必须通过一致性 conformance test（计划书 §16.1）；HDR V2 blob 跨实现合并一致（§11.2）属于 `wavemaker.metrics.v1` 的 conformance 范围。

## 3. 错误码约定（骨架）

统一使用 `wavemaker.common.v1.OperationResult` 包装非流式操作的执行结果（计划书 §5.3 服务草案中 Initialize/ApplyPlan/Start/... 均返回 OperationResult；Phase 1 WM-0103 冻结）。

### 3.1 骨架枚举

```protobuf
enum ErrorCode {
  ERROR_CODE_UNSPECIFIED = 0;
  OK = 1;
  INVALID_ARGUMENT = 2;        // 计划数据、配置参数非法
  FAILED_PRECONDITION = 3;     // 状态机不允许该操作（如未 PlanAccepted 即 Start）
  NOT_FOUND = 4;               // swell_id / backend_id / shard_id 不存在
  ALREADY_EXISTS = 5;          // 重复注册、重复 generation
  PERMISSION_DENIED = 6;
  UNAVAILABLE = 7;             // 依赖组件暂时不可达
  CAPABILITY_UNAVAILABLE = 8;  // Backend 不支持该能力（计划书 §4.3：明确返回，不伪造）
  UNSUPPORTED = 9;             // 协议层面不支持的操作
  ABORTED = 10;                // 操作被 StopAdmission/Drain/取消中断
  INTERNAL = 11;               // 未分类内部错误
}
```

### 3.2 语义规则（源自计划书既有命令语义 §5.4）

- 每个错误必须携带可解释的 `reason_code`（稳定字符串）与 `detail`，不允许只返回数字码（计划书 §8.2.2「校验输出可解释」的协议面延伸）。
- 幂等操作按既定键去重：Initialize 按 generation、ApplyPlan 按 plan_digest、Start/Drain 按 operation_id（§5.4）。重复请求命中幂等键时返回原结果，不是错误。
- 状态转换失败返回 FAILED_PRECONDITION 并携带当前 state 与 generation，不得仅靠命令发送成功推断状态已转换（§4.1.1）。
- 能力不足返回 CAPABILITY_UNAVAILABLE，并附能力目录中的能力名（§5.3.2），与 Unknown/Unsupported/Degraded 能力声明体系对齐。

## 4. 待冻结项

- OperationResult 具体字段（WM-0103）、事件消息 schema（§5.5 事件模型）、指标消息 schema（§5.6 指标模型）在 Phase 1 Protobuf v1 初稿冻结。
- proto 文件级 `option`（go_package、rust prost 配置）随 Rust workspace 建立（WM-0101/0102）确定。

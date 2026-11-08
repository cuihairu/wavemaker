# ADR-008: v1 不含常驻 Dashboard 与多用户管理

> 状态：Accepted（2026-11-09，WM-0012）
> 来源：计划书 Phase 6 前置条件「仅在 v1 产品形态需要时启动」；WM-0600 桌面形态待决事项。

## Context（背景）

WM-0012 要求在 v1 范围冻结前决定：是否需要一个常驻（resident）Dashboard 服务，以及是否需要多用户管理。相关事实：

1. **交付面已经落地**：分发面是 Releases 页的滚动 nightly CLI 二进制（`.github/workflows/nightly.yml`，WM-DB-1..10 已验收）；Phase 1 控制面只暴露 CLI 所需的最小 API（WM-0104 `/healthz`、`/api/v1/status`、gRPC ControlService；WM-0111 `wm run/status/stop`）。
2. **观测面已有形态**：报告是离线工件而非在线服务——`docs/swell.md` 14 节报告结构 + `docs/spec/report-template.md`，由 `wm report` 导出（QPS/TPS 口径已在 WM-0417 登记，待实现）。
3. **多用户是纵向切片**：用户/权限（WM-0607）、审计日志（WM-0608）依赖常驻服务、认证面与存储边界，Phase 1 的控制器没有这些前置。
4. **零常驻是 Phase 1 的既成事实**：tech-stack 明确「CLI 驱动的单次 Swell 只需文件系统 + JSON」（store.rs 模块注释），控制器当前不假设任何浏览器客户端。

## Decision（决策）

**v1 不做常驻 Dashboard，不做多用户管理。**

1. v1 的三个交付面：CLI（nightly 二进制）、报告工件（`wm report` 导出的 JSON/Markdown）、控制 REST/gRPC（仅服务于 CLI 与 Supervisor/Worker 通信）。
2. 单操作员信任模型：v1 部署形态是一台操作机上的一套 CLI + 可选的局部 Controller/Worker/Supervisor 进程；不内置认证、权限、租户概念。暴露到不可信网络不在 v1 范围（安全模型见 `docs/security.md`）。
3. **Phase 6 不随 v1 启动**。WM-0601..0608（Dashboard 页面、用户、审计日志）全部留在 Phase 6，启动前置条件是 WM-0600 明确桌面/仪表盘产品形态（Tauri / Electron / 纯 Web 的取舍，见 todo.md WM-0600 备注）。
4. 若未来引入 Dashboard：形态决策走 WM-0600、权限模型必须新开 ADR；本 ADR 的「无常驻、单操作员」假设届时废止并记录。

## Consequences（后果）

- Phase 1/3/4/5 的验收均以 CLI + 报告工件为准，不设 Web 验收项；「报告含 qps/tps 与曲线」（WM-0417）即 v1 的可视化替代。
- Controller 不需要 session/cookie/token 面向人的认证实现；协议面继续用握手版本拒绝（protobuf-conventions §2.3）与本地信任边界。
- `docs/roadmap.md` Phase 6 保持「仅产品需要时启动」门禁；本 ADR 是该门禁的正式决定记录。
- 多用户、权限、审计能力不得以任何形式提前进入 v1 控制面（避免半成品认证面）。

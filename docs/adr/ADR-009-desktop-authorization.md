# ADR-009: 桌面端用户与角色权限模型（WM-0607）

> 状态：Accepted（2026-11-12，WM-0607）
> 来源：WM-0607（todo.md Phase 6）与 ADR-008 §4 的解除条件。

## Context（背景）

ADR-008 在 v1 范围内冻结了「不做常驻 Dashboard、不做多用户」的决定（单操作员信任模型，无常驻认证面）。其 §4 明确写明：**若未来引入 Dashboard：形态决策走 WM-0600、权限模型必须新开 ADR；本 ADR 的「无常驻、单操作员」假设届时废止并记录。**

WM-0600 已拍板桌面形态 = Tauri（`desktop/`，WM-0601 落地），Phase 6 启动前置解除。WM-0607 要求桌面端加入用户与权限模型：`docs/security.md` 要求「Remote control commands require authentication and authorization」「Externally exposed Controller APIs must have authentication, authorization, audit, and rate limiting」「High-risk actions such as remotely cancelling a Swell or cleaning up bots must record the operator and the reason」。`docs/roadmap.md` WM-0607/WM-0608 把用户、权限、审计列入 Phase 6。

但 ADR-008 同时规定：**多用户、权限、审计能力不得以任何形式提前进入 v1 控制面（避免半成品认证面）**。因此本 ADR 的边界是——**权限检查落在桌面壳与引擎侧，不在 v1 控制面（controller REST/gRPC）上**。

## Decision（决策）

1. **权限面落在桌面壳（`desktop/src-tauri`）与引擎侧决策函数（`wm-desktop` 的 `authz` 模块）**，controller REST/gRPC 保持无认证的单操作员形态（Phase 1 现状不动）。桌面是本地信任边界（本机桌面应用），不引入网络认证面、会话、cookie/token 传输——那属于 Phase 1 控制面之外的未来工作。
2. **三个角色（WM-0607 指定）**：`admin`（全权，含用户管理）、`operator`（日常操作：启停 Swell、清理 bot、应用计划、修改配置/频谱）、`viewer`（只读）。角色即权限矩阵的行；不发明第四个角色。
3. **权限矩阵（每角色 × 每操作，允许/拒绝）**：

   | Action | viewer | operator | admin |
   |---|---|---|---|
   | `view`（查看页面/状态/报告） | ✅ | ✅ | ✅ |
   | `start_swell` | ❌ | ✅ | ✅ |
   | `stop_swell` | ❌ | ✅ | ✅ |
   | `clean_up_bots` | ❌ | ✅ | ✅ |
   | `apply_plan` | ❌ | ✅ | ✅ |
   | `edit_config`（频谱/配置变更） | ❌ | ❌ | ✅ |
   | `manage_users`（用户/角色/口令管理） | ❌ | ❌ | ✅ |

   高风险动作（stop/clean_up/apply_plan）在 WM-0608 的审计里记录操作者与理由（`docs/security.md`）；本 ADR 只定义**谁被允许**，记录由 WM-0608 承担。
4. **登录**：本地用户名 + 口令，口令以加盐哈希（SHA-256，迭代）存入 `users.json`（`docs/security.md`「Tokens, passwords, or account credentials are never written to logs」——只存哈希、绝不写日志、绝不发明传输凭据）。未登录时任何操作均拒绝（`not_authenticated`）。**首次引导（bootstrap）**：空 `users.json` 上的第一次登录即以所输入凭据创建第一个 `admin` 账户并直接登录——否则新安装陷入死锁（`create_user` 需要 admin 会话，而会话需要已有用户）。引导只发生在**空库**上（非空库上的未知用户照常拒绝，不构成接管路径），测试双向钉死。
5. **诚实边界**：`check_permission` 是引擎侧权威决策；前端渲染矩阵与禁用未授权操作只是表现层，判定权始终在 Rust 侧。controller REST 无认证这一事实在 UI 里如实标注（「控制面为本地信任边界，无网络认证」），不发明网络认证通道。

## Consequences（后果）

- ADR-008 §2/§4 的「无常驻、单操作员」假设在**桌面表面**被正式废止并在此记录；ADR-008 对 v1 控制面（REST/gRPC 无认证）的规定**不受影响**——控制器仍按单操作员信任模型运行。
- 权限矩阵是单一事实来源（`authz::permission_matrix()`），测试穷举 3 角色 × 7 操作全部允许/拒绝组合，不允许有漏网的默认放行。
- 口令存储是本地文件哈希，不是面向网络的凭据库；遗忘口令的恢复路径是重建 `users.json`（本机文件，操作员所有）——文档化，不做远程重置。
- WM-0608（审计日志）复用本 ADR 的 `Action` 与操作者身份：审计记录「谁、何时、做了什么」，其中「谁」即登录用户，与角色矩阵正交。

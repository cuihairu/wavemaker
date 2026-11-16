---
title: MCP 服务
---

# MCP 服务

Wavemaker 自带一个 MCP 服务（`wm-mcp`，基于官方
[rmcp](https://crates.io/crates/rmcp) SDK），通过 stdio 把 Controller REST 面、
Spectrum 引擎与测试计划引擎暴露给 AI agent——与 MCP 代理/IDE 走的是同一条链路。

```bash
cargo run -p wm-mcp
```

让 MCP 客户端指向该二进制：

```json
{ "command": "wm-mcp", "args": [] }
```

## 工具

每个工具都委托给既有的引擎面——Controller REST API、`wm` CLI 的命令、或
Runtime 自己的数学。零第二实现，工具因此不可能与引擎实际执行的东西漂移。

| 工具 | 来源 | 说明 |
|---|---|---|
| `list_runs` | Controller REST | 转发 `GET /api/v1/runs/{run_id}`；无 id 时从 `GET /api/v1/status` 内嵌的 `runs` 数组取全量（Controller 没有专门的列表路由）。Controller JSON 原样转发，不重新建模。 |
| `query_status` | Controller REST | 原样转发 `GET /api/v1/status`。 |
| `validate_spectrum` | `wm-cli::commands::validate` | 与 `wm validate` 同一引擎（`wm-core` `SpectrumConfig::validate`），连 CLI 的违规渲染都一致。Error 级问题以工具级错误携报告返回；WARN 级仍算 valid。 |
| `load_timeline` | `wm-runtime` `ArrivalPlan` | 与桌面时间线图同一数学（`rate_at` / `cumulative`）——曲线不可能与 Runtime 实际执行的东西漂移。 |
| `run_scenario` | `wm-cli::commands::run` / `stop` | 库调用进 CLI 子命令面——与 `wm run` / `wm stop` 执行同一代码，类型化错误免 stdout 解析。`start` 先校验 spectrum。 |
| `run_test_plan` | `wm-cli::plan` + `wm-cli::runner` | `wm test run` 引擎：端到端执行声明式测试计划 TOML（步骤、断言、运行目录带 `report.md`）。`failed` verdict 是携原因的成功结果。 |
| `get_test_report` | 运行目录文件 | 回读已完成的计划运行：`run.json`（结构化结果）+ `report.md`（渲染视图）。不给 `run_dir` 时返回 `wm-runs` 下最新一次。 |

stdio e2e 测试断言 `tools/list` **恰好返回这七个**——没有幽灵工具（见下方诚实边界）。

## 资源（Resources）

工具承载状态，不承载知识。配置与词汇类问题——「`arrival.ramp` 是什么意思」
「soak 测试怎么配」——通过 MCP **resources** 回答，让 agent 读权威页面，
而不是凭记忆作答：

| URI | 页面 |
|---|---|
| `wm://docs/configuration` / `wm://docs/configuration.zh` | 配置参考（en / zh-CN） |
| `wm://docs/glossary` | 领域术语表（依其自身约定为中英双语） |
| `wm://docs/spectrum` / `wm://docs/spectrum.zh` | Spectrum 与确定性计划（en / zh-CN） |
| `wm://docs/test-plan` / `wm://docs/test-plan.zh` | 测试计划结构、八种 action、断言（en / zh-CN） |

所有资源均为 `text/markdown`。页面在**构建期**嵌入（`include_str!`）：
二进制携带其构建时那笔 commit 的文档快照，绝不实时读仓库。

## 配置

- **Controller 地址**：默认 `http://127.0.0.1:7600`；可逐工具调用覆盖，或用
  `WM_CONTROLLER_URL` 环境变量——与 `wm` CLI 同一环境变量、同一默认值。
- **测试计划运行**：`backend=stub`（默认）执行进程内模拟器；`backend=controller`
  驱动真实 Controller，capture 需要 `basin_url`。

## 诚实边界

- **实时运行的 metrics / report 不出工具。** 引擎侧尚无数据源
  （WM-0412 / WM-0413 / WM-0416 未实现），出对应工具就是编造数据。
  数据落地之日工具才会出现。因此 `run_test_plan` 在 controller 后端下没有
  `basin_url` 就具名报错，绝不编数；stub 后端的一切都是带标注的模拟。
- **失败即错误**：工具失败返回 `isError` 携具名原因——绝不静默假成功。
  未知资源 URI 返回 MCP `RESOURCE_NOT_FOUND` 错误，绝不编造内容。
- **stdout 纪律**：stdout 只承载 JSON-RPC 帧；诊断信息（如 `run_scenario`
  打印的违规报告）走 stderr。

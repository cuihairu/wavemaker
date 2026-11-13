<p align="center">
  <img src="docs/public/logo.svg" width="64" alt="Wavemaker logo" />
</p>

<h1 align="center">Wavemaker</h1>

<p align="center">跨引擎游戏服务器负载测试框架。</p>

<p align="center"><a href="README.md">English</a> | <a href="README.zh.md">中文</a></p>

<p align="center"><a href="https://github.com/cuihairu/wavemaker/actions/workflows/docs.yml"><img src="https://github.com/cuihairu/wavemaker/actions/workflows/docs.yml/badge.svg" alt="Docs build" /></a> <a href="LICENSE"><img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg" alt="License: Apache-2.0" /></a></p>

## 简介

Wavemaker 通过统一的负载模型（Spectrum）、确定性计划数据（ShardPlan）、可插拔的执行后端接口（Native Runtime 与引擎 Bot 后端）和统一的观测模型（面向服务端的 Basin、面向发生器自身的 Generator Observability），生成、管理并分析大量虚拟客户端对游戏服务端施加的负载，支持跨游戏引擎工作。

它不是单纯的 Bot 脚本框架，也不是通用分布式任务队列。

<p align="center"><img src="docs/public/architecture.svg" alt="architecture"></p>

## 文档

完整技术设计与实施计划发布在 **https://cuihairu.github.io/wavemaker/**，提供英文与简体中文两个版本。内容覆盖总体架构、执行后端模型、KBEngine Bridge 适配器、Spectrum 语义与确定性计划、观测设计、测试策略与分阶段实施计划。

## 状态

Wavemaker 处于持续构建阶段。Phase 0（语义）与 Phase 1（协议 + 最小控制面）已实现并有测试覆盖；文档中的分阶段路线图（Phase 0–6）定义了实现顺序。

MCP 服务器（`crates/wm-mcp`，基于官方 rmcp SDK 的 stdio 传输）向 AI agent 暴露 `list_runs`、`query_status`、`validate_spectrum`、`load_timeline`、`run_scenario`、`run_test_plan` 与 `get_test_report` 七个工具；详见 [crates/wm-mcp/README.md](crates/wm-mcp/README.md)。

## 快速开始

一分钟内跑起第一次负载验证——不需要真实游戏服务器（默认后端是进程内
模拟器，报告醒目标注 `simulated`）：

```bash
cargo build -p wm-cli
./examples/run_demo.sh
```

测试计划是声明式 TOML——注入负载、保持、采集、断言。该脚本把三个示例
计划（1 并发冒烟、10 → 50 → 100 基线阶梯、带泄漏趋势断言的 30 分钟
浸泡）跑在 stub 后端上，各自写出 `wm-runs/<plan>-<时间戳>/report.md`。
单个计划、瞬时完成（真实睡眠压缩、模拟时长全额）：

```bash
wm test run examples/plans/smoke.toml --time-scale 0
```

计划语法、断言语义、controller 后端与 stub 的诚实边界见
[docs/zh/test-plan.md](docs/zh/test-plan.md)。

## 每日构建

最新滚动构建请到 **Releases 页**（`nightly` 发布）获取：

- **https://github.com/cuihairu/wavemaker/releases/tag/nightly**

每天 05:17（北京时间）落一个新构建。按平台选择资产：

| 资产 | 平台 | 验证 |
|---|---|---|
| `wavemaker-linux-x86_64` | Linux x86_64 | `./wavemaker-linux-x86_64 --version` |
| `wavemaker-linux-aarch64` | Linux ARM64 | `./wavemaker-linux-aarch64 --version` |
| `wavemaker-windows-x86_64.zip` | Windows x86_64（`wm.exe`、`wm-mcp.exe`） | 解压后 `./wm.exe --version` |
| `wavemaker-macos-aarch64.tar.gz` | macOS Apple Silicon（`wm`、`wm-mcp`） | `tar -xzf …` 后 `xattr -d com.apple.quarantine wm && ./wm --version` |

每个资产附带 `.sha256` 校验和与 `PLATFORM-NOTES-*.txt`；只保留最新一份构建（旧的会被替换，不会累积）。二进制未签名——SmartScreen/Gatekeeper 可能告警。Linux 两腿是成熟腿；Windows/macOS 两腿目前只验证到编译 + 单测面，尚未在对应平台上对真实游戏服务器做端到端验收（各自的 PLATFORM-NOTES 有说明）。

## 许可证

Apache-2.0，见 [LICENSE](LICENSE)。

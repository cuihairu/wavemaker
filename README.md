<p align="center">
  <img src="docs/public/logo.svg" width="64" alt="Wavemaker logo" />
</p>

<h1 align="center">Wavemaker</h1>

<p align="center">A cross-engine game server load testing framework.</p>

<p align="center"><a href="README.md">English</a> | <a href="README.zh.md">中文</a></p>

<p align="center"><a href="https://github.com/cuihairu/wavemaker/actions/workflows/docs.yml"><img src="https://github.com/cuihairu/wavemaker/actions/workflows/docs.yml/badge.svg" alt="Docs build" /></a> <a href="LICENSE"><img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg" alt="License: Apache-2.0" /></a></p>

## About

Wavemaker generates, manages, and analyzes the load that large numbers of virtual clients apply to game servers. It works across game engines through a unified load model (Spectrum), deterministic plan data (ShardPlan), a pluggable execution backend interface (Native Runtime and engine bot backends), and a unified observability model (Basin for the server, Generator Observability for the load generators themselves).

It is not a plain bot scripting framework, nor a general-purpose distributed task queue.

<p align="center"><img src="docs/public/architecture.svg" alt="architecture"></p>

## Documentation

The complete technical design and implementation plan is published at **https://cuihairu.github.io/wavemaker/**, in English and 简体中文. It covers the overall architecture, execution backend model, the KBEngine Bridge adapter, Spectrum semantics and deterministic plans, observability design, testing strategy, and the phased implementation roadmap.

## Status

Wavemaker is in the active-build stage. Phase 0 (semantics) and Phase 1 (protocol + minimal control plane) are implemented and tested; the phased roadmap (Phase 0–6) in the documentation defines what gets built and in what order.

An MCP server (`crates/wm-mcp`, stdio via the official rmcp SDK) exposes `list_runs`, `query_status`, `validate_spectrum`, `load_timeline`, `run_scenario`, `run_test_plan` and `get_test_report` to AI agents; see [crates/wm-mcp/README.md](crates/wm-mcp/README.md).

## Quickstart

Run a first load verification in under a minute — no game server needed
(the default backend is an in-process simulator; its reports say
`simulated` loudly):

```bash
cargo build -p wm-cli
./examples/run_demo.sh
```

A test plan is a declarative TOML — inject load, hold, capture, assert.
That script runs the three demo plans (a 1-bot smoke, a 10 → 50 → 100
baseline staircase, a 30-minute soak with a leak-trend assertion) against
the stub backend and writes a `wm-runs/<plan>-<timestamp>/report.md` for
each. A single plan, instant (sleeps compressed, simulated durations full):

```bash
wm test run examples/plans/smoke.toml --time-scale 0
```

The plan language, assertion semantics, the controller backend, and the
stub's honest boundaries are in [docs/test-plan.md](docs/test-plan.md).

## Daily Builds

Get the latest rolling build from the **Releases page** (the `nightly` release):

- **https://github.com/cuihairu/wavemaker/releases/tag/nightly**

A new build lands daily (05:17 CST). Choose your asset:

| Asset | Platform | Verify |
|---|---|---|
| `wavemaker-linux-x86_64` | Linux x86_64 | `./wavemaker-linux-x86_64 --version` |
| `wavemaker-linux-aarch64` | Linux ARM64 | `./wavemaker-linux-aarch64 --version` |
| `wavemaker-windows-x86_64.zip` | Windows x86_64 (`wm.exe`, `wm-mcp.exe`) | expand, then `./wm.exe --version` |
| `wavemaker-macos-aarch64.tar.gz` | macOS Apple Silicon (`wm`, `wm-mcp`) | `tar -xzf …` then `xattr -d com.apple.quarantine wm && ./wm --version` |
| `wavemaker-nightly-desktop-linux-x86_64.tar.gz` | Desktop (Tauri GUI), Linux x86_64 | needs webkit2gtk-4.1 — see `DEPENDENCIES.txt` inside; run `./wavemaker-desktop` |
| `wavemaker-nightly-desktop-linux-aarch64.tar.gz` | Desktop (Tauri GUI), Linux ARM64 | same as above |
| `wavemaker-nightly-desktop-windows-x86_64.msi` | Desktop (Tauri GUI), Windows x86_64 | needs WebView2 (preinstalled on Win11) |
| `wavemaker-nightly-desktop-macos-aarch64.dmg` | Desktop (Tauri GUI), macOS Apple Silicon | unsigned — right-click Open on first launch |

Each asset ships with a `.sha256` checksum and a `PLATFORM-NOTES-*.txt`; the latest build is always the only copy (older builds are replaced, never accumulated). Binaries are unsigned — SmartScreen/Gatekeeper may warn. The Linux legs are the mature ones; the Windows/macOS legs are compile + unit-test verified only and are not yet end-to-end accepted against a real game server on those platforms (each PLATFORM-NOTES says so). The desktop builds are a first-run face too: the Linux desktop binary is verified by an xvfb window smoke (the window appears), the Windows/macOS desktop packages by build + package sanity only — none are yet click-tested on a real desktop session.

## License

Apache-2.0. See [LICENSE](LICENSE).

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

## Documentation

The complete technical design and implementation plan is published at **https://cuihairu.github.io/wavemaker/**, in English and 简体中文. It covers the overall architecture, execution backend model, the KBEngine Bridge adapter, Spectrum semantics and deterministic plans, observability design, testing strategy, and the phased implementation roadmap.

## Status

Wavemaker is in the design stage. This repository currently hosts the design documentation and the implementation plan; the phased roadmap (Phase 0–6) in the documentation defines what gets built and in what order. There is nothing to install yet.

## License

Apache-2.0. See [LICENSE](LICENSE).

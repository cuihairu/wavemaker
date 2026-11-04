---
layout: home

hero:
  name: Wavemaker
  text: 跨引擎游戏服务器负载测试框架。
  tagline: 通过统一的负载模型、计划数据、执行后端接口和观测模型，生成、管理并分析大量虚拟客户端对游戏服务端施加的负载。它不是单纯的 Bot 脚本框架，也不是通用分布式任务队列。
  actions:
    - theme: brand
      text: 项目定位
      link: /zh/introduction
    - theme: alt
      text: 实施计划
      link: /zh/roadmap
    - theme: alt
      text: GitHub
      link: https://github.com/cuihairu/wavemaker

features:
  - title: 统一 Spectrum 负载模型
    details: 使用统一 Spectrum 描述压测负载，而不是各引擎各写一套 Bot 脚本。
  - title: Rust Actor Runtime
    details: Wavemaker 自有的 Rust Actor Runtime 作为原生执行后端。
  - title: 复用引擎原生 Bot
    details: 管理和观测游戏引擎已有的原生 Bot。
  - title: 可审计的压测报告
    details: 报告同时呈现数据质量、负载偏差和服务端曲线，而不只是客户端吞吐。
---

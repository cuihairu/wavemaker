---
title: 风险登记与架构质量门
---

# 风险登记

| ID | 风险 | 影响 | 缓解措施 |
|---|---|---|---|
| R-001 | arrival / concurrency 语义不闭合 | 计划错误、无效压测 | Phase 0 数学模型与校验器 |
| R-002 | KBEngine callback 精度不足 | 到达计划偏差 | 实测并声明调度分辨率 |
| R-003 | KBEngine API 不支持精确 Bot 控制 | 能力误报 | Capability Unknown / Unsupported |
| R-004 | Bridge IPC 阻塞引擎线程 | Bot 行为延迟 | 双队列、异步 IO、callback 投递 |
| R-005 | Bridge 事件丢失 | 报告不完整 | sequence、cursor、批量重传 |
| R-006 | Controller / Supervisor 失联 | 状态不一致 | 租约、generation、恢复同步 |
| R-007 | Generator 饱和 | 误判服务端容量 | Generator Saturated 标签 |
| R-008 | HDR 参数不一致 | 分布统计错误 | Controller 下发参数 + conformance |
| R-009 | 时间戳不一致 | 曲线关联错误 | 多时间戳模型、偏移估计、质量标签 |
| R-010 | 容器成为硬依赖 | 部署受限 | Native Process Launcher 优先 |
| R-011 | 多语言协议语义分叉 | 兼容问题 | Protobuf 唯一跨进程契约 |
| R-012 | 过度拆分模块 | 开发成本增加 | 少量 Crate 起步 |
| R-013 | 监控采集影响服务端 | 测量失真 | observer_baseline 和自动降级 |
| R-014 | 混合 Backend 负载不可比 | 结论误导 | 分 Backend 报告与语义校准 |

---

---

# 架构质量门

每个阶段完成时必须回答：

1. 计划是否确定且可重现？
2. Backend 是否如实声明能力？
3. 实际负载是否与计划负载可比较？
4. 发生器是否成为瓶颈？
5. 服务端观测覆盖是否足够？
6. 观测行为是否影响被测系统？
7. 失联和恢复是否会制造额外负载？
8. 数据是否可以追溯到 Swell、Backend、Shard 和时间窗口？
9. 报告是否能明确指出哪些结论可信、哪些数据不完整？
10. 新增抽象是否由真实的第二种实现或明确需求支撑？

任何阶段不得通过“接口已经预留”代替可运行的 conformance test。

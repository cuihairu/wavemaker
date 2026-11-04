---
title: Swell 生命周期与报告
---

# Swell 生命周期与报告

## 13.1 Swell 阶段

建议：

```text
Created
 -> Validating
 -> Preflight
 -> Calibrating
 -> Preparing
 -> RampUp
 -> Hold
 -> RampDown
 -> Draining
 -> Collecting
 -> Reporting
 -> Completed

任意阶段 -> Failed / Cancelled / Invalid
```

阶段边界必须写入事件时间线。

## 13.2 报告结构

1. Experiment Summary
2. Spectrum / Load Profile
3. Planned vs Actual Arrival
4. Concurrency Curve
5. Client Performance
6. Generator Health
7. Engine Bot Backend Details
8. Server Performance（Basin）
9. Correlation Analysis
10. Bottleneck Analysis
11. Data Quality
12. Test Validity
13. Environment / Version / Configuration
14. Raw Data References

## 13.3 报告结论约束

- 客户端延迟不能替代服务端 Tick 指标。
- CPU 高不自动证明 CPU 是唯一瓶颈。
- 指标曲线相关不等于因果。
- 发生器饱和时，不应给出服务端最大容量结论。
- 监控覆盖不足时，必须降低结论置信度。
- 不同 Backend 结果应可分组查看。

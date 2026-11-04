---
title: Swell Lifecycle & Reports
---

# Swell Lifecycle and Reports

## Swell Phases

Suggested:

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

Any phase -> Failed / Cancelled / Invalid
```

Phase boundaries must be written into the event timeline.

## Report Structure

1. Experiment Summary
2. Spectrum / Load Profile
3. Planned vs Actual Arrival
4. Concurrency Curve
5. Client Performance
6. Generator Health
7. Engine Bot Backend Details
8. Server Performance (Basin)
9. Correlation Analysis
10. Bottleneck Analysis
11. Data Quality
12. Test Validity
13. Environment / Version / Configuration
14. Raw Data References

## Constraints on Report Conclusions

- Client latency does not substitute for server-side tick metrics.
- High CPU does not automatically prove CPU is the only bottleneck.
- Correlated metric curves do not imply causation.
- When the generator is saturated, no conclusion about maximum server capacity should be given.
- When monitoring coverage is insufficient, conclusion confidence must be lowered.
- Results from different Backends should be viewable in groups.

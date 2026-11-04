---
title: Generator Observability
---

# Generator Observability and Data Quality

## The Generator Itself Must Be Monitored

Every Worker / Supervisor / Engine Bot Process records:

- CPU, RSS, thread count, FD.
- Event queue length.
- Control channel latency.
- Number of dropped event reports.
- Metrics reporting latency.
- Deviation between planned_at and actual_at.
- Scheduler lag.
- Actor / bot count.
- Active connection count.
- Process restarts and lost-contact events.

When the generator hits a resource bottleneck, the report must be marked Generator Saturated; a generator bottleneck must not be misread as a server-side bottleneck.

## HDR Histogram

- All Workers use the same sigfigs, lowest, highest, and epoch definitions.
- Parameters are issued by the Controller and written into plan metadata.
- When V2 histoblob is transported as opaque bytes, it must carry schema/version/parameter digests.
- Averaging P99 values across Workers is not allowed.
- Mergeable histograms aggregate the overall distribution.
- Coordinated omission correction happens on the Worker side.
- Cross-language implementations must pass conformance tests with the same input, same parameters, same output.

## Data Quality Labels

Reports must be able to show:

- Complete
- Partial
- Delayed
- Missing
- Estimated
- Invalid
- GeneratorSaturated
- ObserverDegraded

Missing data must not default to zero.

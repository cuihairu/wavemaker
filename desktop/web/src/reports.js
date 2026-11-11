// Engine-defined report sections for the report comparison page (WM-0606).
//
// The section list mirrors the authoritative `CorrelationReport` type in
// wm-basin (crates/wm-basin/src/report.rs): swell_id, timeline_anchor,
// effective_window, timeline alignment, generator aggregate, saturation
// signals, per-series server aggregates, quality report and labels.
// Nothing here is invented.
//
// Report CONTENT is currently ABSENT: no report artifact is persisted
// anywhere (no `report.json`/`report.yaml` writer in the workspace), the
// controller exposes no report endpoint (`/api/v1/status` + `/api/v1/runs`
// only), and the correlation-report deliverable (WM-0416) is not a
// completed roadmap item yet. The view therefore compares the run records
// that WOULD carry a report (real `/status` data) and documents the report
// structure, rather than fabricating report numbers.

/// The report sections a comparison would show, in the engine's field order.
export const REPORT_SECTIONS = [
  { section: 'swell_id', content: 'the swell the report was built for' },
  { section: 'timeline_anchor', content: 'anchor the single timeline was built on' },
  { section: 'effective_window', content: 'declared correlation range ∩ source coverage' },
  { section: 'timeline', content: 'WM-0414 alignment record (clocks, trusts, offsets)' },
  { section: 'generator', content: 'WM-0412 aggregate of the generator plane' },
  { section: 'saturation', content: 'saturation evidence ("Generator Saturated")' },
  { section: 'server', content: 'WM-0413 per-series server aggregates' },
  { section: 'quality', content: 'WM-0408 missing-data and delay detection' },
  { section: 'labels', content: 'WM-0415 data-quality labels' },
]

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
/// The per-section descriptions are user-visible text, so they live in the
/// i18n dictionaries (en wording in i18n/locales/en.js) and the view renders
/// `t(contentKey)`.
export const REPORT_SECTIONS = [
  { section: 'swell_id', contentKey: 'reports.sectionSwellId' },
  { section: 'timeline_anchor', contentKey: 'reports.sectionTimelineAnchor' },
  { section: 'effective_window', contentKey: 'reports.sectionEffectiveWindow' },
  { section: 'timeline', contentKey: 'reports.sectionTimeline' },
  { section: 'generator', contentKey: 'reports.sectionGenerator' },
  { section: 'saturation', contentKey: 'reports.sectionSaturation' },
  { section: 'server', contentKey: 'reports.sectionServer' },
  { section: 'quality', contentKey: 'reports.sectionQuality' },
  { section: 'labels', contentKey: 'reports.sectionLabels' },
]

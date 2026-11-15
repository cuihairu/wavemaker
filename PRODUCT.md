# Wavemaker — Product Context

## What Wavemaker is
Wavemaker is a load-testing control plane for game-server infrastructure. It models load as
"swells" (waves of traffic) driven through a spectrum config, executed by workers under a
controller, with metrics, reports, users/roles, and an append-only audit trail.

Name origin: ocean waves — a swell is a defined traffic wave profile; runs are wave instances.

## Who uses it
- **SRE / performance engineers** who define load spectra, launch runs, watch progress live,
  and compare reports after a soak.
- **Game-server operators** checking that a server build survives a traffic profile before
  release (often paired with KBEngine-based backends).
- **Team leads** reviewing audit trail and user/permission setup.

Session shape: bursts of focused work — configure a spectrum, launch a run, watch status,
then leave it running for hours; come back to compare reports. The UI is checked repeatedly
in short intervals, often alongside terminals.

## The real scene
A performance engineer sits with terminals open, runs Wavemaker in a browser beside them,
glances at run status between other tasks. The screen is often on a second monitor showing
live status for extended periods. Light conditions: normal office/monitor lighting, no extreme
dark-room or outdoor usage pattern. Both light and dark usage happen; the tool should default
to a calm dark terminal-adjacent feel without being a terminal costume.

## What the product must prove
- **Operational truth at a glance**: is a run healthy, degrading, or failing — in seconds,
  from across the desk (status colors legible at a glance).
- **Instrument credibility**: numbers, charts, and states must look like measured data, not
  decoration. This is a tool an engineer trusts during a load incident.
- **Calm under load**: the UI itself must never feel like it is panicking; severity is
  communicated by precise, restrained color, not alarm-red everywhere.

## Constraints
- Vue 3 + Vite, no CSS framework installed; views are small SFCs (~50-200 lines) with
  data-test attributes used by vitest e2e tests. Redesign must keep data-test attributes
  and view routes intact.
- Backend is the Rust controller (`wm-controller-server`, HTTP :7600). The web UI reads
  `/api/v1/status` and friends; demo data acceptable only where clearly labeled.
- Bilingual UI (en / zh-CN via vue-i18n; glossary terms stay English per docs/glossary.md);
  docs are English-first with a zh README mirror.
- Must remain keyboard-reachable and readable; audit/users tables stay functional.
- Brand: no logo asset exists; the word-mark "Wavemaker" in type only. Waves/swells are the
  product's own metaphor and may inform visual identity — but the UI is a working instrument,
  not an ocean poster.

// Controller REST client + pure swell-derivation helpers (WM-0601).
//
// The desktop webview talks to the controller through the Tauri HTTP
// plugin so requests go out from the Rust side and never trip webview
// CORS. Outside Tauri (vitest / a plain browser) it falls back to the
// global fetch so the module stays testable.

import { fetch as tauriFetch } from '@tauri-apps/plugin-http'
import { invoke as tauriInvoke } from '@tauri-apps/api/core'

export const CONTROLLER_URL =
  import.meta.env.VITE_CONTROLLER_URL || 'http://127.0.0.1:7600'

const inTauri =
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

const httpFetch = inTauri ? tauriFetch : globalThis.fetch

/// GET /api/v1/status. Throws a named error on a non-OK response —
/// never silently returns an empty status.
export async function fetchStatus() {
  const response = await httpFetch(`${CONTROLLER_URL}/api/v1/status`)
  if (!response.ok) {
    throw new Error(`controller_status_failed: ${response.status}`)
  }
  return response.json()
}

/// Group the controller's runs into swells. Pure and deterministic:
/// one entry per distinct swell_id, counts summed, states counted,
/// earliest creation kept, ordered newest-first.
export function deriveSwells(runs) {
  const bySwell = new Map()
  for (const run of runs) {
    const existing = bySwell.get(run.swell_id)
    if (existing) {
      existing.run_count += 1
      existing.states[run.state] = (existing.states[run.state] || 0) + 1
      if (run.created_at < existing.created_at_min) {
        existing.created_at_min = run.created_at
      }
      existing.runs.push(run)
    } else {
      bySwell.set(run.swell_id, {
        swell_id: run.swell_id,
        run_count: 1,
        states: { [run.state]: 1 },
        created_at_min: run.created_at,
        runs: [run],
      })
    }
  }
  return [...bySwell.values()].sort((a, b) => b.created_at_min - a.created_at_min)
}

/// Render a counted-states map deterministically: `Created: 1, Stopped: 2`.
export function formatStates(states) {
  return Object.entries(states)
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([state, count]) => `${state}: ${count}`)
    .join(', ')
}

/// Worker rows for the status page (WM-0602). Each worker keeps its
/// controller payload verbatim plus one derived flag: `expired` mirrors
/// the controller's own `expired_workers` verdict — the UI never guesses
/// heartbeat thresholds. Ordered by worker_id for a stable table.
export function deriveWorkerRows(status) {
  const expired = new Set(status.expired_workers || [])
  return (status.workers || [])
    .map((worker) => ({ ...worker, expired: expired.has(worker.worker_id) }))
    .sort((a, b) => a.worker_id.localeCompare(b.worker_id))
}

/// Invoke a Tauri command defined in the desktop shell (WM-0603).
/// Outside Tauri (vitest / a plain browser) it rejects with a named
/// error so callers render it like any other failure.
async function desktopInvoke(cmd, args) {
  if (!inTauri) {
    throw new Error('desktop_unavailable')
  }
  return tauriInvoke(cmd, args)
}

/// Spectrum file access goes through shell commands that read/write the
/// file from the Rust side; validation reuses wm-core's `wm validate`
/// engine, so the report matches the CLI exactly.
export const loadSpectrum = (path) => desktopInvoke('load_spectrum_file', { path })
export const saveSpectrum = (path, content) =>
  desktopInvoke('save_spectrum_file', { path, content })
export const validateSpectrum = (content) => desktopInvoke('validate_spectrum', { content })

/// Deterministic issue count: `1 error, 2 warnings`.
export function summarizeIssues(issues) {
  const errors = issues.filter((i) => i.severity === 'error').length
  const warnings = issues.filter((i) => i.severity === 'warning').length
  return `${errors} error${errors === 1 ? '' : 's'}, ${warnings} warning${warnings === 1 ? '' : 's'}`
}

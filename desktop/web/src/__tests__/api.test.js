import { describe, it, expect } from 'vitest'
import { deriveSwells, formatStates, deriveWorkerRows } from '../api.js'

const run = (overrides = {}) => ({
  run_id: 'r1',
  swell_id: 'sw-1',
  spectrum_path: 'spec.yaml',
  state: 'Created',
  created_at: 1000,
  ...overrides,
})

describe('deriveSwells', () => {
  it('returns [] for no runs', () => {
    expect(deriveSwells([])).toEqual([])
  })

  it('groups runs by swell_id and counts states honestly', () => {
    const swells = deriveSwells([
      run({ run_id: 'r1', swell_id: 'sw-1', state: 'Created', created_at: 1000 }),
      run({ run_id: 'r2', swell_id: 'sw-1', state: 'Stopped', created_at: 2000 }),
      run({ run_id: 'r3', swell_id: 'sw-2', state: 'Created', created_at: 500 }),
    ])
    expect(swells).toHaveLength(2)
    const sw1 = swells.find((s) => s.swell_id === 'sw-1')
    expect(sw1.run_count).toBe(2)
    expect(sw1.states).toEqual({ Created: 1, Stopped: 1 })
    expect(sw1.created_at_min).toBe(1000)
    expect(sw1.runs).toHaveLength(2)
  })

  it('keeps each swell\'s runs verbatim and sorts newest-first', () => {
    const swells = deriveSwells([
      run({ run_id: 'old', swell_id: 'old-sw', created_at: 100 }),
      run({ run_id: 'new', swell_id: 'new-sw', created_at: 900 }),
    ])
    expect(swells[0].swell_id).toBe('new-sw')
    expect(swells[1].swell_id).toBe('old-sw')
    expect(swells[0].runs[0].run_id).toBe('new')
  })
})

describe('formatStates', () => {
  it('renders counted states deterministically', () => {
    expect(formatStates({ Created: 1, Stopped: 1 })).toBe('Created: 1, Stopped: 1')
    expect(formatStates({})).toBe('')
  })
})

describe('deriveWorkerRows', () => {
  const worker = (worker_id, overrides = {}) => ({
    worker_id,
    generation: 1,
    capabilities: ['native'],
    adapter_version: '0.1',
    runtime_version: '0.1',
    seconds_since_heartbeat: 0,
    ...overrides,
  })

  it('returns [] when no workers are registered', () => {
    expect(deriveWorkerRows({ workers: [], expired_workers: [] })).toEqual([])
    expect(deriveWorkerRows({})).toEqual([])
  })

  it('flags expiry from the controller verdict and keeps payload verbatim', () => {
    const rows = deriveWorkerRows({
      workers: [worker('w-a'), worker('w-b', { seconds_since_heartbeat: 9 })],
      expired_workers: ['w-b'],
    })
    expect(rows).toHaveLength(2)
    expect(rows[0].worker_id).toBe('w-a')
    expect(rows[0].expired).toBe(false)
    expect(rows[1].worker_id).toBe('w-b')
    expect(rows[1].expired).toBe(true)
    expect(rows[1].seconds_since_heartbeat).toBe(9)
  })

  it('sorts rows by worker_id for a stable table', () => {
    const rows = deriveWorkerRows({
      workers: [worker('w-c'), worker('w-a'), worker('w-b')],
      expired_workers: [],
    })
    expect(rows.map((r) => r.worker_id)).toEqual(['w-a', 'w-b', 'w-c'])
  })
})

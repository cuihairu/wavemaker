import { describe, it, expect } from 'vitest'
import { deriveSwells, formatStates } from '../api.js'

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

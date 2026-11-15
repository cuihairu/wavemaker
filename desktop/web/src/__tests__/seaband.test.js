import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import { createRouter, createMemoryHistory } from 'vue-router'
import SeaBand from '../components/SeaBand.vue'
import { deriveSwells } from '../api.js'
import { routes } from '../router.js'

// SeaBand is presentational: it receives the derived swell list and
// renders one of its three states. No network boundary to mock.
function makeRouter() {
  return createRouter({ history: createMemoryHistory(), routes })
}

const runs = [
  { run_id: 'r1', swell_id: 'sw-1', spectrum_path: 'a.yaml', state: 'Created', created_at: 1000 },
  { run_id: 'r2', swell_id: 'sw-1', spectrum_path: 'b.yaml', state: 'Stopped', created_at: 2000 },
  { run_id: 'r3', swell_id: 'sw-2', spectrum_path: 'c.yaml', state: 'Created', created_at: 500 },
]

describe('SeaBand', () => {
  it('renders one layer and legend entry per swell with active runs', () => {
    const wrapper = mount(SeaBand, {
      props: { swells: deriveSwells(runs) },
      global: { plugins: [makeRouter()] },
    })
    // Two swells exist but sw-1 has one Stopped run — only live runs count.
    const items = wrapper.findAll('[data-test="band-legend-item"]')
    expect(items).toHaveLength(2)
    expect(items[0].text()).toContain('sw-1')
    expect(items[0].text()).toContain('1 active')
    // The band canvas draws one sea layer per active swell.
    expect(wrapper.findAll('.sea-layer')).toHaveLength(2)
    expect(wrapper.find('[data-test="band-empty"]').exists()).toBe(false)
  })

  it('calms to the empty state with the Spectrum launch link when nothing is active', () => {
    // All runs stopped — the sea is honestly calm, not fake-active.
    const wrapper = mount(SeaBand, {
      props: { swells: deriveSwells([{ ...runs[1] }]) },
      global: { plugins: [makeRouter()] },
    })
    const empty = wrapper.find('[data-test="band-empty"]')
    expect(empty.text()).toContain('No active swells')
    expect(empty.find('a[href="/spectrum"]').exists()).toBe(true)
    expect(wrapper.findAll('.sea-layer')).toHaveLength(0)
  })

  it('ripples as a skeleton while the status is loading', () => {
    const wrapper = mount(SeaBand, {
      props: { swells: null },
      global: { plugins: [makeRouter()] },
    })
    expect(wrapper.find('[data-test="band-loading"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="band-empty"]').exists()).toBe(false)
  })
})

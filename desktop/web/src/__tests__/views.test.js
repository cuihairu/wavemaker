import { describe, it, expect, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createRouter, createMemoryHistory } from 'vue-router'
import SwellListView from '../views/SwellListView.vue'
import SwellDetailView from '../views/SwellDetailView.vue'
import { routes } from '../router.js'

// Mock only the network call; the pure derivation helpers stay real so
// the views are exercised end to end over real grouping.
vi.mock('../api.js', async (importOriginal) => {
  const actual = await importOriginal()
  return { ...actual, fetchStatus: vi.fn() }
})

import { fetchStatus } from '../api.js'

const runs = [
  { run_id: 'r1', swell_id: 'sw-1', spectrum_path: 'a.yaml', state: 'Created', created_at: 1000 },
  { run_id: 'r2', swell_id: 'sw-1', spectrum_path: 'b.yaml', state: 'Stopped', created_at: 2000 },
  { run_id: 'r3', swell_id: 'sw-2', spectrum_path: 'c.yaml', state: 'Created', created_at: 500 },
]

function makeRouter() {
  return createRouter({ history: createMemoryHistory(), routes })
}

describe('SwellListView', () => {
  it('renders derived swell rows from the controller status', async () => {
    fetchStatus.mockResolvedValue({ runs })
    const wrapper = mount(SwellListView, { global: { plugins: [makeRouter()] } })
    await flushPromises()
    const rows = wrapper.findAll('[data-test="swell-row"]')
    expect(rows).toHaveLength(2)
    // Newest swell first.
    expect(rows[0].text()).toContain('sw-1')
    expect(rows[0].text()).toContain('2')
    expect(rows[1].text()).toContain('sw-2')
  })

  it('renders the empty state honestly (no fabricated swells)', async () => {
    fetchStatus.mockResolvedValue({ runs: [] })
    const wrapper = mount(SwellListView, { global: { plugins: [makeRouter()] } })
    await flushPromises()
    expect(wrapper.find('[data-test="empty"]').text()).toBe('No swells yet.')
  })

  it('surfaces a controller error by name', async () => {
    // The controller's status call rejects. Settle it only after the
    // component has mounted and attached its await handler, so vitest's
    // rejection detector never sees a dangling rejection.
    let rejectStatus
    fetchStatus.mockImplementation(
      () => new Promise((_resolve, reject) => { rejectStatus = reject })
    )
    const wrapper = mount(SwellListView, {
      global: { stubs: { RouterLink: { template: '<a><slot /></a>' } } },
    })
    await new Promise((resolve) => setTimeout(resolve, 0))
    rejectStatus(new Error('controller_status_failed: 500'))
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(wrapper.find('[data-test="error"]').text()).toContain('controller_status_failed')
  })
})

describe('SwellDetailView', () => {
  it('shows only the selected swell\'s runs', async () => {
    fetchStatus.mockResolvedValue({ runs })
    const router = makeRouter()
    router.push('/swell/sw-1')
    await router.isReady()
    const wrapper = mount(SwellDetailView, { global: { plugins: [router] } })
    await flushPromises()
    expect(wrapper.find('h1').text()).toBe('sw-1')
    const rows = wrapper.findAll('[data-test="run-row"]')
    expect(rows).toHaveLength(2)
    expect(rows[0].text()).toContain('r1')
    expect(rows[0].text()).toContain('a.yaml')
  })

  it('shows an honest empty state for a swell with no runs', async () => {
    fetchStatus.mockResolvedValue({ runs })
    const router = makeRouter()
    router.push('/swell/sw-ghost')
    await router.isReady()
    const wrapper = mount(SwellDetailView, { global: { plugins: [router] } })
    await flushPromises()
    expect(wrapper.find('[data-test="empty"]').text()).toBe('No runs for this swell.')
  })
})

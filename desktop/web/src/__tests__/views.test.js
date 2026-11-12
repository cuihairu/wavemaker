import { describe, it, expect, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createRouter, createMemoryHistory } from 'vue-router'
import SwellListView from '../views/SwellListView.vue'
import SwellDetailView from '../views/SwellDetailView.vue'
import StatusView from '../views/StatusView.vue'
import SpectrumView from '../views/SpectrumView.vue'
import MetricsView from '../views/MetricsView.vue'
import ReportsView from '../views/ReportsView.vue'
import UsersView from '../views/UsersView.vue'
import LoginView from '../views/LoginView.vue'
import { GENERATOR_METRICS, SERVER_METRICS } from '../metrics.js'
import { REPORT_SECTIONS } from '../reports.js'
import { routes } from '../router.js'

// Mock only the shell boundary (network call + Tauri commands); the
// pure derivation helpers stay real so the views are exercised end to
// end over real grouping.
vi.mock('../api.js', async (importOriginal) => {
  const actual = await importOriginal()
  return {
    ...actual,
    fetchStatus: vi.fn(),
    loadSpectrum: vi.fn(),
    saveSpectrum: vi.fn(),
    validateSpectrum: vi.fn(),
    timelineFor: vi.fn(),
    permissionMatrix: vi.fn(),
    currentSession: vi.fn(),
    login: vi.fn(),
    logout: vi.fn(),
    listUsers: vi.fn(),
    createUser: vi.fn(),
    deleteUser: vi.fn(),
    setUserRole: vi.fn(),
  }
})

import {
  fetchStatus,
  loadSpectrum,
  saveSpectrum,
  validateSpectrum,
  timelineFor,
  permissionMatrix,
  currentSession,
  login,
  listUsers,
  createUser,
  deleteUser,
  setUserRole,
} from '../api.js'

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

describe('StatusView', () => {
  const workerPayload = {
    worker_id: 'w-b',
    generation: 3,
    capabilities: ['native', 'kbengine'],
    adapter_version: '0.1',
    runtime_version: '0.1',
    seconds_since_heartbeat: 2,
  }

  it('renders backend summary and worker rows from the controller status', async () => {
    fetchStatus.mockResolvedValue({
      status: 'running',
      protocol_version: 1,
      worker_count: 2,
      expired_workers: ['w-b'],
      workers: [
        { ...workerPayload, worker_id: 'w-a' },
        workerPayload,
      ],
      runs,
    })
    const wrapper = mount(StatusView, { global: { plugins: [makeRouter()] } })
    await flushPromises()
    expect(wrapper.find('[data-test="backend-state"]').text()).toContain('running')
    expect(wrapper.find('[data-test="backend-worker-count"]').text()).toContain('2')
    // Rows sorted by worker_id; expiry mirrors the controller verdict only.
    const rows = wrapper.findAll('[data-test="worker-row"]')
    expect(rows).toHaveLength(2)
    expect(rows[0].text()).toContain('w-a')
    expect(rows[0].text()).toContain('Active')
    expect(rows[1].text()).toContain('w-b')
    expect(rows[1].text()).toContain('Expired')
    expect(rows[1].text()).toContain('2s ago')
    // Contract-missing fields render as unwired, never fabricated values.
    expect(wrapper.find('[data-test="unwired-note"]').text()).toContain('Not wired')
    expect(rows[0].text()).toContain('Not wired')
  })

  it('renders an honest empty state when no workers are registered', async () => {
    fetchStatus.mockResolvedValue({
      status: 'running',
      protocol_version: 1,
      worker_count: 0,
      expired_workers: [],
      workers: [],
      runs: [],
    })
    const wrapper = mount(StatusView, { global: { plugins: [makeRouter()] } })
    await flushPromises()
    expect(wrapper.find('[data-test="empty"]').text()).toBe('No workers registered.')
    expect(wrapper.find('[data-test="backend-state"]').text()).toContain('running')
  })

  it('surfaces a controller error by name', async () => {
    let rejectStatus
    fetchStatus.mockImplementation(
      () => new Promise((_resolve, reject) => { rejectStatus = reject })
    )
    const wrapper = mount(StatusView, {
      global: { stubs: { RouterLink: { template: '<a><slot /></a>' } } },
    })
    await new Promise((resolve) => setTimeout(resolve, 0))
    rejectStatus(new Error('controller_status_failed: 500'))
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(wrapper.find('[data-test="error"]').text()).toContain('controller_status_failed')
  })
})

describe('SpectrumView', () => {
  const YAML = 'arrival:\n  ramp:\n    from_per_second: 1\n'

  it('loads a spectrum file into the editor', async () => {
    loadSpectrum.mockResolvedValue(YAML)
    const wrapper = mount(SpectrumView, { global: { plugins: [makeRouter()] } })
    await wrapper.find('[data-test="path-input"]').setValue('/tmp/spectrum.yaml')
    await wrapper.find('[data-test="load"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-test="editor"]').element.value).toBe(YAML)
    expect(wrapper.find('[data-test="error"]').exists()).toBe(false)
  })

  it('renders validation issues from the wm-core report verbatim', async () => {
    validateSpectrum.mockResolvedValue({
      ok: false,
      parse_error: null,
      issues: [
        {
          severity: 'error',
          code: 'E_POLICY_MISSING',
          field: 'saturation_policy',
          message: 'concurrency.max is set but saturation_policy is missing',
        },
        {
          severity: 'warning',
          code: 'W_TARGET_NAIVE_LITTLE_DEVIATION',
          field: 'concurrency.target',
          message: 'naive λ·W deviates',
        },
      ],
    })
    const wrapper = mount(SpectrumView, { global: { plugins: [makeRouter()] } })
    await wrapper.find('[data-test="editor"]').setValue(YAML)
    await wrapper.find('[data-test="validate"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-test="issue-summary"]').text()).toBe('1 error, 1 warning')
    const issues = wrapper.findAll('[data-test="issue"]')
    expect(issues).toHaveLength(2)
    expect(issues[0].text()).toContain('E_POLICY_MISSING')
    expect(issues[0].text()).toContain('saturation_policy')
    expect(wrapper.find('[data-test="issue-severity"]').text()).toBe('error')
    expect(wrapper.find('[data-test="valid"]').exists()).toBe(false)
  })

  it('shows a valid config without hiding warnings', async () => {
    validateSpectrum.mockResolvedValue({
      ok: true,
      parse_error: null,
      issues: [{ severity: 'warning', code: 'W_X', field: 'f', message: 'm' }],
    })
    const wrapper = mount(SpectrumView, { global: { plugins: [makeRouter()] } })
    await wrapper.find('[data-test="validate"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-test="valid"]').text()).toBe('Config is valid.')
    expect(wrapper.find('[data-test="issue-summary"]').text()).toBe('0 errors, 1 warning')
  })

  it('surfaces a parse error from the shell validator', async () => {
    validateSpectrum.mockResolvedValue({
      ok: false,
      parse_error: 'invalid YAML at line 1',
      issues: [],
    })
    const wrapper = mount(SpectrumView, { global: { plugins: [makeRouter()] } })
    await wrapper.find('[data-test="validate"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-test="parse-error"]').text()).toContain('invalid YAML at line 1')
    expect(wrapper.findAll('[data-test="issue"]')).toHaveLength(0)
  })

  it('confirms a save with the written path', async () => {
    saveSpectrum.mockResolvedValue(undefined)
    const wrapper = mount(SpectrumView, { global: { plugins: [makeRouter()] } })
    await wrapper.find('[data-test="path-input"]').setValue('/tmp/spectrum.yaml')
    await wrapper.find('[data-test="save"]').trigger('click')
    await flushPromises()
    expect(saveSpectrum).toHaveBeenCalledWith('/tmp/spectrum.yaml', '')
    expect(wrapper.find('[data-test="saved"]').text()).toBe('saved /tmp/spectrum.yaml')
  })

  it('surfaces a named shell error when loading fails', async () => {
    loadSpectrum.mockRejectedValue(new Error('cannot read /nope: not found'))
    const wrapper = mount(SpectrumView, { global: { plugins: [makeRouter()] } })
    await wrapper.find('[data-test="load"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-test="error"]').text()).toContain('cannot read /nope')
  })
})

describe('MetricsView', () => {
  const statusPayload = {
    status: 'running',
    protocol_version: 1,
    worker_count: 2,
    expired_workers: [],
    workers: [
      { worker_id: 'w-a', generation: 1, capabilities: ['native'] },
      { worker_id: 'w-b', generation: 1, capabilities: ['native'] },
    ],
    runs,
  }

  it('renders the live summary and every engine-defined metric family honestly', async () => {
    fetchStatus.mockResolvedValue(statusPayload)
    const wrapper = mount(MetricsView, { global: { plugins: [makeRouter()] } })
    await flushPromises()
    expect(wrapper.find('[data-test="backend-state"]').text()).toContain('running')
    expect(wrapper.find('[data-test="worker-count"]').text()).toBe('2')
    expect(wrapper.find('[data-test="run-count"]').text()).toBe('3')
    // Generator: one row per engine metric, each source honestly unwired.
    const genRows = wrapper.findAll('[data-test="generator-metric-row"]')
    expect(genRows).toHaveLength(GENERATOR_METRICS.length)
    expect(genRows[0].text()).toContain('cpu_ratio')
    expect(genRows[0].text()).toContain('Not wired')
    expect(genRows[11].text()).toContain('active_connections')
    // Server: one row per catalog kind.
    const srvRows = wrapper.findAll('[data-test="server-metric-row"]')
    expect(srvRows).toHaveLength(SERVER_METRICS.length)
    expect(srvRows[0].text()).toContain('gauge')
    expect(srvRows[0].text()).toContain('Not wired')
  })

  it('explains why no curves are drawn, without fabricating a data source', async () => {
    fetchStatus.mockResolvedValue(statusPayload)
    const wrapper = mount(MetricsView, { global: { plugins: [makeRouter()] } })
    await flushPromises()
    const note = wrapper.find('[data-test="unwired-note"]').text()
    expect(note).toContain('no metrics endpoint')
    expect(note).toContain('WM-0412')
    expect(note).toContain('nothing is fabricated')
    // No <svg> chart is rendered when there is no series to plot.
    expect(wrapper.findAll('svg')).toHaveLength(0)
  })

  it('renders the summary even when no workers are registered', async () => {
    fetchStatus.mockResolvedValue({ ...statusPayload, worker_count: 0, workers: [] })
    const wrapper = mount(MetricsView, { global: { plugins: [makeRouter()] } })
    await flushPromises()
    expect(wrapper.find('[data-test="worker-count"]').text()).toBe('0')
  })

  it('surfaces a controller error by name', async () => {
    let rejectStatus
    fetchStatus.mockImplementation(
      () => new Promise((_resolve, reject) => { rejectStatus = reject })
    )
    const wrapper = mount(MetricsView, {
      global: { stubs: { RouterLink: { template: '<a><slot /></a>' } } },
    })
    await new Promise((resolve) => setTimeout(resolve, 0))
    rejectStatus(new Error('controller_status_failed: 500'))
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(wrapper.find('[data-test="error"]').text()).toContain('controller_status_failed')
  })
})

describe('ReportsView', () => {
  const compareRunsPayload = [
    { run_id: 'r1', swell_id: 'sw-1', spectrum_path: 'a.yaml', state: 'Created', created_at: 1000 },
    { run_id: 'r2', swell_id: 'sw-2', spectrum_path: 'b.yaml', state: 'Completed', created_at: 2000 },
    { run_id: 'r3', swell_id: 'sw-1', spectrum_path: 'c.yaml', state: 'Stopped', created_at: 500 },
  ]

  it('compares the two selected runs side by side from real run data', async () => {
    fetchStatus.mockResolvedValue({ runs: compareRunsPayload })
    const wrapper = mount(ReportsView, { global: { plugins: [makeRouter()] } })
    await flushPromises()
    // Default selection is the first two runs (newest-first).
    expect(wrapper.find('[data-test="head-a"]').text()).toBe('r1')
    expect(wrapper.find('[data-test="head-b"]').text()).toBe('r2')
    const rows = wrapper.findAll('[data-test="comparison-row"]')
    expect(rows).toHaveLength(5)
    expect(rows[0].text()).toContain('r1')
    expect(rows[1].text()).toContain('sw-1')
    expect(rows[2].text()).toContain('a.yaml')
    expect(rows[3].text()).toContain('Created')
    expect(rows[3].text()).toContain('Completed')
  })

  it('documents the report sections the engine defines', async () => {
    fetchStatus.mockResolvedValue({ runs: compareRunsPayload })
    const wrapper = mount(ReportsView, { global: { plugins: [makeRouter()] } })
    await flushPromises()
    const sections = wrapper.findAll('[data-test="report-section"]')
    expect(sections).toHaveLength(REPORT_SECTIONS.length)
    expect(sections[0].text()).toContain('swell_id')
  })

  it('explains why report content is not compared, without fabricating it', async () => {
    fetchStatus.mockResolvedValue({ runs: compareRunsPayload })
    const wrapper = mount(ReportsView, { global: { plugins: [makeRouter()] } })
    await flushPromises()
    const note = wrapper.find('[data-test="unwired-note"]').text()
    expect(note).toContain('no report file is persisted')
    expect(note).toContain('WM-0416')
    expect(note).toContain('nothing is fabricated')
  })

  it('shows an honest empty state when there are no runs', async () => {
    fetchStatus.mockResolvedValue({ runs: [] })
    const wrapper = mount(ReportsView, { global: { plugins: [makeRouter()] } })
    await flushPromises()
    expect(wrapper.find('[data-test="empty"]').text()).toContain('No runs to compare')
    // The honest note still explains the missing report content.
    expect(wrapper.find('[data-test="unwired-note"]').text()).toContain('WM-0416')
  })

  it('surfaces a controller error by name', async () => {
    let rejectStatus
    fetchStatus.mockImplementation(
      () => new Promise((_resolve, reject) => { rejectStatus = reject })
    )
    const wrapper = mount(ReportsView, {
      global: { stubs: { RouterLink: { template: '<a><slot /></a>' } } },
    })
    await new Promise((resolve) => setTimeout(resolve, 0))
    rejectStatus(new Error('controller_status_failed: 500'))
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(wrapper.find('[data-test="error"]').text()).toContain('controller_status_failed')
  })
})

describe('UsersView', () => {
  // Shape identical to the shell's permission_matrix() rows (WM-0607).
  const matrix = [
    { action: 'view', viewer: true, operator: true, admin: true, minimum_role: null },
    { action: 'stop_swell', viewer: false, operator: true, admin: true, minimum_role: 'operator' },
    { action: 'edit_config', viewer: false, operator: false, admin: true, minimum_role: 'admin' },
  ]
  const admin = { username: 'ada', role: 'admin' }

  function mountUsers(session) {
    permissionMatrix.mockResolvedValue(matrix)
    currentSession.mockResolvedValue(session)
    return mount(UsersView, { global: { plugins: [makeRouter()] } })
  }

  it('renders the permission matrix verbatim from the shell', async () => {
    currentSession.mockResolvedValue(null)
    permissionMatrix.mockResolvedValue(matrix)
    listUsers.mockRejectedValue(new Error('not_authenticated'))
    const wrapper = mount(UsersView, { global: { plugins: [makeRouter()] } })
    await flushPromises()
    const rows = wrapper.findAll('[data-test="matrix-row"]')
    expect(rows).toHaveLength(matrix.length)
    expect(rows[0].text()).toContain('view')
    expect(rows[0].text()).toContain('allow')
    expect(rows[1].text()).toContain('deny')
    // The honest control-plane note is present.
    expect(wrapper.find('[data-test="control-plane-note"]').text()).toContain('single-operator')
  })

  it('shows users and management UI for an admin session', async () => {
    listUsers.mockResolvedValue([
      { username: 'ada', role: 'admin' },
      { username: 'bob', role: 'viewer' },
    ])
    const wrapper = mountUsers(admin)
    await flushPromises()
    expect(wrapper.find('[data-test="session-info"]').text()).toContain('ada')
    const rows = wrapper.findAll('[data-test="user-row"]')
    expect(rows).toHaveLength(2)
    expect(wrapper.find('[data-test="create-form"]').exists()).toBe(true)
  })

  it('denies management to non-admins without hiding the matrix', async () => {
    const wrapper = mountUsers({ username: 'bob', role: 'viewer' })
    await flushPromises()
    expect(wrapper.find('[data-test="manage-denied"]').text()).toContain('admin')
    expect(wrapper.find('[data-test="create-form"]').exists()).toBe(false)
  })

  it('creates a user and refreshes the list', async () => {
    listUsers.mockResolvedValue([])
    createUser.mockResolvedValue({ username: 'carol', role: 'operator' })
    const wrapper = mountUsers(admin)
    await flushPromises()
    await wrapper.find('[data-test="new-username"]').setValue('carol')
    await wrapper.find('[data-test="new-password"]').setValue('pw')
    await wrapper.find('[data-test="new-role"]').setValue('operator')
    await wrapper.find('[data-test="create-form"]').trigger('submit')
    await flushPromises()
    expect(createUser).toHaveBeenCalledWith('carol', 'pw', 'operator')
    expect(wrapper.find('[data-test="notice"]').text()).toContain('carol')
  })

  it('surfaces a named denial when a management call fails', async () => {
    listUsers.mockResolvedValue([{ username: 'bob', role: 'viewer' }])
    deleteUser.mockRejectedValue(new Error('forbidden: role viewer may not manage_users (requires admin)'))
    const wrapper = mountUsers(admin)
    await flushPromises()
    await wrapper.find('[data-test="delete-bob"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-test="error"]').text()).toContain('forbidden: role viewer')
  })
})

describe('LoginView', () => {
  it('logs in, sets the session and returns to the swell list', async () => {
    login.mockResolvedValue({ username: 'ada', role: 'admin' })
    const router = makeRouter()
    router.push('/login')
    await router.isReady()
    const wrapper = mount(LoginView, {
      global: { plugins: [router] },
    })
    await wrapper.find('[data-test="username-input"]').setValue('ada')
    await wrapper.find('[data-test="password-input"]').setValue('pw')
    // jsdom does not turn a submit-button click into a form submission.
    await wrapper.find('form').trigger('submit')
    await flushPromises()
    expect(login).toHaveBeenCalledWith('ada', 'pw')
    expect(wrapper.find('[data-test="error"]').exists()).toBe(false)
    expect(router.currentRoute.value.path).toBe('/')
  })

  it('renders the named shell error on a failed login', async () => {
    login.mockRejectedValue(new Error('login_failed: BadPassword'))
    const wrapper = mount(LoginView, { global: { plugins: [makeRouter()] } })
    await wrapper.find('[data-test="username-input"]').setValue('ada')
    await wrapper.find('[data-test="password-input"]').setValue('wrong')
    await wrapper.find('form').trigger('submit')
    await flushPromises()
    expect(wrapper.find('[data-test="error"]').text()).toContain('login_failed: BadPassword')
    // Stays on the login page.
    expect(wrapper.find('[data-test="local-note"]').exists()).toBe(true)
  })
})

describe('SpectrumView timeline', () => {
  const timelinePayload = {
    ramp_duration_secs: 600,
    horizon_secs: 1200,
    points: [
      { t_secs: 0, rate_per_second: 1, expected_arrivals: 0 },
      { t_secs: 600, rate_per_second: 20, expected_arrivals: 3800.5 },
      { t_secs: 1200, rate_per_second: 20, expected_arrivals: 15800.5 },
    ],
  }

  it('renders two single-series charts and a sample table from the engine curve', async () => {
    timelineFor.mockResolvedValue(timelinePayload)
    const wrapper = mount(SpectrumView, { global: { plugins: [makeRouter()] } })
    await wrapper.find('[data-test="timeline"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-test="timeline-window"]').text()).toContain('1200')
    expect(wrapper.findAll('svg path[stroke]')).toHaveLength(2)
    expect(wrapper.find('[data-test="rate-chart"]').text()).toContain('Arrival rate')
    expect(wrapper.find('[data-test="cumulative-chart"]').text()).toContain('Expected arrivals')
    // Strided table subset is labeled, never presented as complete.
    expect(wrapper.find('[data-test="timeline-section"]').text()).toContain('every 10th sample')
    expect(wrapper.find('[data-test="timeline-note"]').text()).toContain('No concurrency curve')
  })

  it('surfaces a named shell error when the timeline cannot be built', async () => {
    timelineFor.mockRejectedValue(
      new Error('cannot build timeline: spectrum has no arrival section')
    )
    const wrapper = mount(SpectrumView, { global: { plugins: [makeRouter()] } })
    await wrapper.find('[data-test="timeline"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-test="error"]').text()).toContain('no arrival section')
    expect(wrapper.find('[data-test="timeline-section"]').exists()).toBe(false)
  })
})

// English UI strings — the source of truth for wording. Protocol tokens
// (run states, role names, error identifiers) intentionally stay out of
// this file: they come from the controller payload and render verbatim.
export default {
  nav: {
    swells: 'Swells',
    status: 'Status',
    spectrum: 'Spectrum',
    metrics: 'Metrics',
    reports: 'Reports',
    users: 'Users',
    audit: 'Audit',
  },
  shell: {
    login: 'Log in',
    logout: 'Log out',
  },
  back: {
    allSwells: '← All swells',
  },
  common: {
    loading: 'Loading…',
    retry: 'Retry',
    notWired: 'Not wired',
    workerActive: 'Active',
    workerExpired: 'Expired',
    secondsAgo: '{n}s ago',
  },
  errors: {
    devDown:
      'Controller is not running — run `pnpm dev:full` (or `cargo run -p wm-controller --bin wm-controller-server`) from desktop/web, then retry.',
    shellDown: 'The desktop service is not responding — restart the app, then retry.',
  },
  sealand: {
    heading: 'Sea state',
    note: 'now — latest composition; history scrubbing lands with the metrics series (WM-0412)',
    empty: 'No active swells — {link}',
    emptyLaunch: 'launch from Spectrum',
    activeRuns: '{n} active',
    unwiredRps: '— RPS',
    tooltipRps: 'no live RPS series yet (WM-0412)',
    nowTag: 'now',
    ariaOverview: 'Active swell overview',
    ariaComposition: 'Stacked composition of active swells',
  },
  swells: {
    heading: 'Swells',
    noSwell: 'No swells yet.',
    colWave: 'Wave',
    colSwell: 'Swell',
    colState: 'State',
    colRuns: 'Runs',
    colRps: 'RPS now',
    colP99: 'p99',
    colErrors: 'Errors',
    colStarted: 'Started',
    tooltipRps: 'no live RPS series yet (WM-0412)',
    tooltipP99: 'no latency data source yet (WM-0412)',
    tooltipErrors: 'no error data source yet (WM-0412)',
    ariaMark: 'runs share mark for {id}',
    unwiredNote:
      'RPS now, p99 and error counts have no controller data source yet (metrics face, WM-0412) — shown as "—", never fabricated. The wave mark encodes each swell\'s share of total runs.',
  },
  login: {
    heading: 'Log in',
    username: 'Username',
    password: 'Password',
    localNote:
      'Accounts are local to this desktop (users file with salted password hashes); the controller REST surface has no network authentication (ADR-008, ADR-009). If no users are registered yet, the first login creates the admin account.',
  },
  status: {
    heading: 'Backend / Worker status',
    backend: 'Backend',
    state: 'State',
    protocolVersion: 'Protocol version',
    connectedWorkers: 'Connected workers',
    expiredWorkers: 'Expired workers',
    address: 'Address',
    throughputQueue: 'Throughput / queue',
    workers: 'Workers',
    noWorkers: 'No workers registered.',
    colWorker: 'Worker',
    colActorLoad: 'Actor load',
    colHeartbeat: 'Last heartbeat',
    colErrors: 'Errors',
    colCapabilities: 'Capabilities',
    colGeneration: 'Generation',
    unwiredNote:
      'Fields marked "Not wired" have no controller data source yet (backend address, throughput/queue, actor load, error counts).',
  },
}

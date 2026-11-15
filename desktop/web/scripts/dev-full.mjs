#!/usr/bin/env node
// One-command dev stack (WM-DEV-001): start the controller
// (wm-controller-server via cargo run), wait for its /healthz, then run
// vite. If a controller already answers /healthz it is reused instead of
// being started twice. Ctrl-C tears both processes down.
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const webDir = path.dirname(path.dirname(fileURLToPath(import.meta.url)))
const HEALTH_URL = 'http://127.0.0.1:7600/healthz'
const POLL_MS = 1000
// The first `cargo run` may compile for minutes; be generous before
// giving up so a cold tree does not turn into a spurious failure.
const HEALTH_TIMEOUT_MS = 10 * 60 * 1000

function log(msg) {
  console.log(`[dev:full] ${msg}`)
}

async function healthz() {
  try {
    const res = await fetch(HEALTH_URL, { signal: AbortSignal.timeout(2000) })
    return res.ok ? await res.text() : null
  } catch {
    return null
  }
}

async function waitForController() {
  const deadline = Date.now() + HEALTH_TIMEOUT_MS
  while (Date.now() < deadline) {
    if ((await healthz()) === 'ok') return true
    await new Promise((resolve) => setTimeout(resolve, POLL_MS))
  }
  return false
}

function killTree(child) {
  if (!child || child.exitCode !== null) return
  child.kill('SIGTERM')
  // The controller only handles Ctrl-C; SIGTERM's default disposition is
  // enough, with a SIGKILL backstop for anything that lingers.
  const killer = setTimeout(() => child.kill('SIGKILL'), 1500)
  child.once('exit', () => clearTimeout(killer))
}

// Reuse an already-healthy controller instead of failing on the port.
let controller = null
let vite = null
if ((await healthz()) === 'ok') {
  log('controller already healthy on :7600 — reusing it')
} else {
  log('starting wm-controller-server via cargo run (first compile can take a while)…')
  controller = spawn('cargo', ['run', '-p', 'wm-controller', '--bin', 'wm-controller-server'], {
    stdio: 'inherit',
    cwd: path.resolve(webDir, '../..'),
  })
  controller.on('exit', (code) => {
    if (vite === null || vite.exitCode === null) {
      log(`controller exited early (code ${code}) — stopping`)
      process.exit(1)
    }
  })
  if (!(await waitForController())) {
    log('controller never became healthy on :7600 — giving up')
    killTree(controller)
    process.exit(1)
  }
}
log('controller healthy')

vite = spawn('pnpm', ['exec', 'vite'], { stdio: 'inherit', cwd: webDir })

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    killTree(vite)
    killTree(controller)
    process.exit(0)
  })
}

vite.on('exit', (code) => {
  log(`vite exited (code ${code})`)
  killTree(controller)
  process.exit(code ?? 0)
})

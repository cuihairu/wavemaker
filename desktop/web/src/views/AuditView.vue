<script setup>
// Audit trail page (WM-0608). Renders the append-only trail of gated
// attempts — who / when / what, allowed or denied — verbatim from the
// shell. Filtering happens on the Rust side (query_audit): the form is
// passed through, the UI never filters locally and never invents rows.
// The query itself is a `view` attempt and is recorded like any other.
import { onMounted, ref } from 'vue'
import { queryAudit } from '../api.js'

// The engine matrix's actions (ADR-009) — the only values the Rust
// query accepts; anything else is a named bad-filter error.
const ACTIONS = [
  'view',
  'start_swell',
  'stop_swell',
  'clean_up_bots',
  'apply_plan',
  'edit_config',
  'manage_users',
]

const entries = ref([])
const error = ref('')
const queried = ref(false)
const username = ref('')
const action = ref('all')
const outcome = ref('all')

function buildFilter() {
  return {
    username: username.value.trim() || null,
    action: action.value === 'all' ? null : action.value,
    outcome: outcome.value === 'all' ? null : outcome.value,
  }
}

async function runQuery(filter) {
  error.value = ''
  try {
    entries.value = await queryAudit(filter)
    queried.value = true
  } catch (err) {
    error.value = err.message || String(err)
    entries.value = []
    queried.value = false
  }
}

function time(isoMs) {
  return new Date(isoMs).toISOString()
}

onMounted(() => runQuery({}))
</script>

<template>
  <main>
    <RouterLink to="/">← All swells</RouterLink>
    <h1>Audit trail</h1>
    <p v-if="error" class="error" data-test="error">{{ error }}</p>

    <form data-test="filter-form" @submit.prevent="runQuery(buildFilter())">
      <p>
        <label>
          Username
          <input v-model="username" data-test="filter-username" />
        </label>
        <label>
          Action
          <select v-model="action" data-test="filter-action">
            <option value="all">all</option>
            <option v-for="a in ACTIONS" :key="a" :value="a">{{ a }}</option>
          </select>
        </label>
        <label>
          Outcome
          <select v-model="outcome" data-test="filter-outcome">
            <option value="all">all</option>
            <option value="allowed">allowed</option>
            <option value="denied">denied</option>
          </select>
        </label>
        <button type="submit" data-test="filter-apply">Query</button>
      </p>
    </form>

    <p v-if="queried && entries.length === 0 && !error" data-test="empty">
      No audit entries match.
    </p>
    <table v-else-if="queried" data-test="audit-table">
      <thead>
        <tr>
          <th>#</th>
          <th>Time (UTC)</th>
          <th>User</th>
          <th>Action</th>
          <th>Outcome</th>
          <th>Detail</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="entry in entries" :key="entry.seq" data-test="audit-row">
          <td>{{ entry.seq }}</td>
          <td>{{ time(entry.timestamp_ms) }}</td>
          <td>{{ entry.username || '—' }}</td>
          <td><code>{{ entry.action }}</code></td>
          <td>{{ entry.outcome }}</td>
          <td>{{ entry.detail }}</td>
        </tr>
      </tbody>
    </table>

    <p class="unwired-note" data-test="audit-note">
      The trail records every gated attempt in the desktop shell (allowed
      and denied), append-only — nothing rewrites or deletes entries, and
      each query is itself recorded as a view attempt. Controller
      operations (start/stop/clean up/apply plan) are not wired through
      the shell yet, so they have no entries here; logins are session
      events, not matrix actions (ADR-008, ADR-009).
    </p>
  </main>
</template>

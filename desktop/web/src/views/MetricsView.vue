<script setup>
// Generator / Server metric curves page (WM-0605). Same shell shape as
// the Status view: one `fetchStatus()` on mount, named error, honest
// states. The metric families come from the engine's own types (see
// metrics.js) and every one is rendered with an honest "Not wired"
// source — the controller exposes no metrics endpoint and no metric
// series is persisted yet (WM-0412/0413 pending), so a curve would be
// fabricated data. The wired summary below is real `/status` data.
import { onMounted, ref } from 'vue'
import { fetchStatus } from '../api.js'
import { GENERATOR_METRICS, SERVER_METRICS } from '../metrics.js'

const status = ref(null)
const error = ref('')

onMounted(async () => {
  try {
    status.value = await fetchStatus()
  } catch (err) {
    error.value = err.message || String(err)
  }
})
</script>

<template>
  <main>
    <RouterLink to="/">← All swells</RouterLink>
    <h1>Generator / Server metrics</h1>
    <p v-if="error" class="error" data-test="error">{{ error }}</p>
    <p v-else-if="status === null" data-test="loading">Loading…</p>
    <template v-else>
      <section data-test="summary-section">
        <h2>Live summary</h2>
        <table data-test="summary-table">
          <tbody>
            <tr>
              <th scope="row">Backend state</th>
              <td data-test="backend-state">{{ status.status }}</td>
            </tr>
            <tr>
              <th scope="row">Connected workers</th>
              <td data-test="worker-count">{{ status.worker_count }}</td>
            </tr>
            <tr>
              <th scope="row">Runs</th>
              <td data-test="run-count">{{ (status.runs || []).length }}</td>
            </tr>
          </tbody>
        </table>
        <p class="muted">
          Worker roster and per-worker details live on the
          <RouterLink to="/status">Status</RouterLink> page.
        </p>
      </section>

      <section data-test="generator-section">
        <h2>Generator</h2>
        <table data-test="generator-table">
          <thead>
            <tr>
              <th>Metric</th>
              <th>Kind</th>
              <th>Aggregation</th>
              <th>Unit</th>
              <th>Curve source</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="m in GENERATOR_METRICS"
              :key="m.metric"
              data-test="generator-metric-row"
            >
              <td><code>{{ m.metric }}</code></td>
              <td>{{ m.kind }}</td>
              <td>{{ m.aggregate }}</td>
              <td>{{ m.unit }}</td>
              <td><span class="unwired">Not wired</span></td>
            </tr>
          </tbody>
        </table>
      </section>

      <section data-test="server-section">
        <h2>Server</h2>
        <table data-test="server-table">
          <thead>
            <tr>
              <th>Kind</th>
              <th>Aggregation</th>
              <th>Rule</th>
              <th>Curve source</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="s in SERVER_METRICS"
              :key="s.kind"
              data-test="server-metric-row"
            >
              <td>{{ s.kind }}</td>
              <td>{{ s.aggregate }}</td>
              <td class="muted">{{ s.note }}</td>
              <td><span class="unwired">Not wired</span></td>
            </tr>
          </tbody>
        </table>
      </section>

      <p class="unwired-note" data-test="unwired-note">
        No metric curves are shown: the controller exposes no metrics
        endpoint (only <code>/api/v1/status</code>), no metric series is
        persisted, and the generator/server aggregation pipeline
        (WM-0412 / WM-0413) is not a completed deliverable yet. These
        are the metric families the engine defines — a curve appears
        here once a metrics surface is wired, and nothing is fabricated.
      </p>
    </template>
  </main>
</template>

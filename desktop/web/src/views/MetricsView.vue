<script setup>
// Generator / Server metric curves page (WM-0605). Same shell shape as
// the Status view: one `fetchStatus()` on mount, named error, honest
// states. The metric families come from the engine's own types (see
// metrics.js) and every one is rendered with an honest "Not wired"
// source — the controller exposes no metrics endpoint and no metric
// series is persisted yet (WM-0412/0413 pending), so a curve would be
// fabricated data. The wired summary below is real `/status` data.
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { fetchStatus } from '../api.js'
import { GENERATOR_METRICS, SERVER_METRICS } from '../metrics.js'

const { t } = useI18n()

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
    <RouterLink to="/">{{ t('back.allSwells') }}</RouterLink>
    <h1>{{ t('metrics.heading') }}</h1>
    <p v-if="error" class="error" data-test="error">{{ error }}</p>
    <p v-else-if="status === null" data-test="loading">{{ t('common.loading') }}</p>
    <template v-else>
      <section data-test="summary-section">
        <h2>{{ t('metrics.liveSummary') }}</h2>
        <table data-test="summary-table">
          <tbody>
            <tr>
              <th scope="row">{{ t('metrics.backendState') }}</th>
              <td data-test="backend-state">{{ status.status }}</td>
            </tr>
            <tr>
              <th scope="row">{{ t('status.connectedWorkers') }}</th>
              <td data-test="worker-count">{{ status.worker_count }}</td>
            </tr>
            <tr>
              <th scope="row">{{ t('metrics.runsRow') }}</th>
              <td data-test="run-count">{{ (status.runs || []).length }}</td>
            </tr>
          </tbody>
        </table>
        <p class="muted">
          <i18n-t keypath="metrics.statusHint" scope="global">
            <template #link><RouterLink to="/status">{{ t('nav.status') }}</RouterLink></template>
          </i18n-t>
        </p>
      </section>

      <section data-test="generator-section">
        <h2>{{ t('metrics.generator') }}</h2>
        <table data-test="generator-table">
          <thead>
            <tr>
              <th>{{ t('metrics.colMetric') }}</th>
              <th>{{ t('metrics.colKind') }}</th>
              <th>{{ t('metrics.colAggregation') }}</th>
              <th>{{ t('metrics.colUnit') }}</th>
              <th>{{ t('metrics.colSource') }}</th>
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
              <td><span class="unwired">{{ t('common.notWired') }}</span></td>
            </tr>
          </tbody>
        </table>
      </section>

      <section data-test="server-section">
        <h2>{{ t('metrics.server') }}</h2>
        <table data-test="server-table">
          <thead>
            <tr>
              <th>{{ t('metrics.colKind') }}</th>
              <th>{{ t('metrics.colAggregation') }}</th>
              <th>{{ t('metrics.colRule') }}</th>
              <th>{{ t('metrics.colSource') }}</th>
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
              <td class="muted">{{ t(s.noteKey) }}</td>
              <td><span class="unwired">{{ t('common.notWired') }}</span></td>
            </tr>
          </tbody>
        </table>
      </section>

      <p class="unwired-note" data-test="unwired-note">
        <i18n-t keypath="metrics.unwiredNote" scope="global">
          <template #statusEndpoint><code>/api/v1/status</code></template>
        </i18n-t>
      </p>
    </template>
  </main>
</template>

<script setup>
// Backend / Worker status page (WM-0602). Same shape as the Swell
// views: one `fetchStatus()` on mount, named error, honest empty state.
// Only controller-contract fields are rendered; anything the REST API
// does not carry yet shows "Not wired" instead of a fabricated value.
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { fetchStatus, deriveWorkerRows, stateTone } from '../api.js'

const { t } = useI18n()

const status = ref(null)
const workers = ref(null)
const error = ref('')

onMounted(async () => {
  try {
    status.value = await fetchStatus()
    workers.value = deriveWorkerRows(status.value)
  } catch (err) {
    error.value = err.message || String(err)
  }
})
</script>

<template>
  <main>
    <RouterLink to="/">{{ t('back.allSwells') }}</RouterLink>
    <h1>{{ t('status.heading') }}</h1>
    <p v-if="error" class="error" data-test="error">{{ error }}</p>
    <p v-else-if="status === null" data-test="loading">{{ t('common.loading') }}</p>
    <template v-else>
      <section data-test="backend-section">
        <h2>{{ t('status.backend') }}</h2>
        <table data-test="backend-table">
          <tbody>
            <tr data-test="backend-state">
              <th scope="row">{{ t('status.state') }}</th>
              <td>{{ status.status }}</td>
            </tr>
            <tr>
              <th scope="row">{{ t('status.protocolVersion') }}</th>
              <td>{{ status.protocol_version }}</td>
            </tr>
            <tr data-test="backend-worker-count">
              <th scope="row">{{ t('status.connectedWorkers') }}</th>
              <td>{{ status.worker_count }}</td>
            </tr>
            <tr>
              <th scope="row">{{ t('status.expiredWorkers') }}</th>
              <td>{{ (status.expired_workers || []).length }}</td>
            </tr>
            <tr>
              <th scope="row">{{ t('status.address') }}</th>
              <td><span class="unwired">{{ t('common.notWired') }}</span></td>
            </tr>
            <tr>
              <th scope="row">{{ t('status.throughputQueue') }}</th>
              <td><span class="unwired">{{ t('common.notWired') }}</span></td>
            </tr>
          </tbody>
        </table>
      </section>
      <section data-test="workers-section">
        <h2>{{ t('status.workers') }}</h2>
        <p v-if="workers.length === 0" data-test="empty">{{ t('status.noWorkers') }}</p>
        <table v-else data-test="worker-table">
          <thead>
            <tr>
              <th>{{ t('status.colWorker') }}</th>
              <th>{{ t('status.colState') }}</th>
              <th>{{ t('status.colActorLoad') }}</th>
              <th>{{ t('status.colHeartbeat') }}</th>
              <th>{{ t('status.colErrors') }}</th>
              <th>{{ t('status.colCapabilities') }}</th>
              <th>{{ t('status.colGeneration') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="worker in workers" :key="worker.worker_id" data-test="worker-row">
              <td>{{ worker.worker_id }}</td>
              <td>
                <span
                  class="pill"
                  :class="`pill--${stateTone(worker.expired ? 'Expired' : 'Active')}`"
                >
                  {{ worker.expired ? t('common.workerExpired') : t('common.workerActive') }}
                </span>
              </td>
              <td><span class="unwired">{{ t('common.notWired') }}</span></td>
              <td>{{ t('common.secondsAgo', { n: worker.seconds_since_heartbeat }) }}</td>
              <td><span class="unwired">{{ t('common.notWired') }}</span></td>
              <td>{{ worker.capabilities.join(', ') }}</td>
              <td>{{ worker.generation }}</td>
            </tr>
          </tbody>
        </table>
        <p class="unwired-note" data-test="unwired-note">
          {{ t('status.unwiredNote') }}
        </p>
      </section>
    </template>
  </main>
</template>

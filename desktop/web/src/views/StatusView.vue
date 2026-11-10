<script setup>
// Backend / Worker status page (WM-0602). Same shape as the Swell
// views: one `fetchStatus()` on mount, named error, honest empty state.
// Only controller-contract fields are rendered; anything the REST API
// does not carry yet shows "Not wired" instead of a fabricated value.
import { onMounted, ref } from 'vue'
import { fetchStatus, deriveWorkerRows } from '../api.js'

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
    <RouterLink to="/">← All swells</RouterLink>
    <h1>Backend / Worker status</h1>
    <p v-if="error" class="error" data-test="error">{{ error }}</p>
    <p v-else-if="status === null" data-test="loading">Loading…</p>
    <template v-else>
      <section data-test="backend-section">
        <h2>Backend</h2>
        <table data-test="backend-table">
          <tbody>
            <tr data-test="backend-state">
              <th scope="row">State</th>
              <td>{{ status.status }}</td>
            </tr>
            <tr>
              <th scope="row">Protocol version</th>
              <td>{{ status.protocol_version }}</td>
            </tr>
            <tr data-test="backend-worker-count">
              <th scope="row">Connected workers</th>
              <td>{{ status.worker_count }}</td>
            </tr>
            <tr>
              <th scope="row">Expired workers</th>
              <td>{{ (status.expired_workers || []).length }}</td>
            </tr>
            <tr>
              <th scope="row">Address</th>
              <td><span class="unwired">Not wired</span></td>
            </tr>
            <tr>
              <th scope="row">Throughput / queue</th>
              <td><span class="unwired">Not wired</span></td>
            </tr>
          </tbody>
        </table>
      </section>
      <section data-test="workers-section">
        <h2>Workers</h2>
        <p v-if="workers.length === 0" data-test="empty">No workers registered.</p>
        <table v-else data-test="worker-table">
          <thead>
            <tr>
              <th>Worker</th>
              <th>State</th>
              <th>Actor load</th>
              <th>Last heartbeat</th>
              <th>Errors</th>
              <th>Capabilities</th>
              <th>Generation</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="worker in workers" :key="worker.worker_id" data-test="worker-row">
              <td>{{ worker.worker_id }}</td>
              <td>{{ worker.expired ? 'Expired' : 'Active' }}</td>
              <td><span class="unwired">Not wired</span></td>
              <td>{{ worker.seconds_since_heartbeat }}s ago</td>
              <td><span class="unwired">Not wired</span></td>
              <td>{{ worker.capabilities.join(', ') }}</td>
              <td>{{ worker.generation }}</td>
            </tr>
          </tbody>
        </table>
        <p class="unwired-note" data-test="unwired-note">
          Fields marked "Not wired" have no controller data source yet
          (backend address, throughput/queue, actor load, error counts).
        </p>
      </section>
    </template>
  </main>
</template>

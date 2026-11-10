<script setup>
import { onMounted, ref } from 'vue'
import { fetchStatus, deriveSwells, formatStates } from '../api.js'

const swells = ref(null)
const error = ref('')

onMounted(async () => {
  try {
    const status = await fetchStatus()
    swells.value = deriveSwells(status.runs || [])
  } catch (err) {
    error.value = err.message || String(err)
  }
})
</script>

<template>
  <main>
    <h1>Swells</h1>
    <p v-if="error" class="error" data-test="error">{{ error }}</p>
    <p v-else-if="swells === null" data-test="loading">Loading…</p>
    <p v-else-if="swells.length === 0" data-test="empty">No swells yet.</p>
    <table v-else data-test="swell-table">
      <thead>
        <tr>
          <th>Swell</th>
          <th>Runs</th>
          <th>States</th>
          <th>Earliest created</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="swell in swells" :key="swell.swell_id" data-test="swell-row">
          <td>
            <RouterLink :to="`/swell/${encodeURIComponent(swell.swell_id)}`">
              {{ swell.swell_id }}
            </RouterLink>
          </td>
          <td>{{ swell.run_count }}</td>
          <td>{{ formatStates(swell.states) }}</td>
          <td>{{ new Date(swell.created_at_min * 1000).toLocaleString() }}</td>
        </tr>
      </tbody>
    </table>
  </main>
</template>

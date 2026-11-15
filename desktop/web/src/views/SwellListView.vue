<script setup>
import { onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { fetchStatus, deriveSwells, formatStates } from '../api.js'
import SeaBand from '../components/SeaBand.vue'

const swells = ref(null)
const error = ref('')
const router = useRouter()

async function load() {
  error.value = ''
  swells.value = null
  try {
    const status = await fetchStatus()
    swells.value = deriveSwells(status.runs || [])
  } catch (err) {
    error.value = err.message || String(err)
  }
}

onMounted(load)

function openSwell(swellId) {
  router.push(`/swell/${encodeURIComponent(swellId)}`)
}
</script>

<template>
  <main>
    <h1>Swells</h1>
    <SeaBand v-if="!error" :swells="swells" @open="openSwell" />
    <div v-if="error" class="error error-card" data-test="error">
      <span>{{ error }}</span>
      <button data-test="retry" @click="load">Retry</button>
    </div>
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
          <td class="num">{{ swell.run_count }}</td>
          <td>{{ formatStates(swell.states) }}</td>
          <td class="num">{{ new Date(swell.created_at_min * 1000).toLocaleString() }}</td>
        </tr>
      </tbody>
    </table>
  </main>
</template>

<style scoped>
.error {
  margin-top: 16px;
}

table {
  margin-top: 20px;
}
</style>

<script setup>
import { onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { fetchStatus, stateTone } from '../api.js'

const { t } = useI18n()
const route = useRoute()
const swellId = decodeURIComponent(route.params.swellId)
const runs = ref(null)
const error = ref('')

onMounted(async () => {
  try {
    const status = await fetchStatus()
    runs.value = (status.runs || []).filter((run) => run.swell_id === swellId)
  } catch (err) {
    error.value = err.message || String(err)
  }
})
</script>

<template>
  <main>
    <RouterLink to="/">{{ t('back.allSwells') }}</RouterLink>
    <h1>{{ swellId }}</h1>
    <p v-if="error" class="error" data-test="error">{{ error }}</p>
    <p v-else-if="runs === null" data-test="loading">{{ t('common.loading') }}</p>
    <p v-else-if="runs.length === 0" data-test="empty">{{ t('swellDetail.noRuns') }}</p>
    <table v-else data-test="run-table">
      <thead>
        <tr>
          <th>{{ t('swellDetail.colRun') }}</th>
          <th>{{ t('swellDetail.colState') }}</th>
          <th>{{ t('swellDetail.colSpectrum') }}</th>
          <th>{{ t('swellDetail.colCreated') }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="run in runs" :key="run.run_id" data-test="run-row">
          <td>{{ run.run_id }}</td>
          <td>
            <span class="pill" :class="`pill--${stateTone(run.state)}`">{{ run.state }}</span>
          </td>
          <td>{{ run.spectrum_path }}</td>
          <td>{{ new Date(run.created_at * 1000).toLocaleString() }}</td>
        </tr>
      </tbody>
    </table>
  </main>
</template>

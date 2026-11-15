<script setup>
// Report comparison page (WM-0606). The report CONTENT the engine defines
// (see reports.js) has no persisted or served artifact yet — no report
// file is written anywhere and the controller exposes no report endpoint
// (WM-0416 pending). So the honest comparison here is over the real run
// records that WOULD carry a report: two runs from the controller's
// `/api/v1/status` compared field by field. Nothing is fabricated.
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { fetchStatus, compareRuns } from '../api.js'
import { REPORT_SECTIONS } from '../reports.js'

const { t } = useI18n()

const runs = ref([])
const error = ref('')
const loaded = ref(false)
const selectedA = ref('')
const selectedB = ref('')

onMounted(async () => {
  try {
    const status = await fetchStatus()
    runs.value = status.runs || []
    // Default selection: the first two runs, newest-first like the list.
    selectedA.value = runs.value[0]?.run_id || ''
    selectedB.value = runs.value[1]?.run_id || ''
    loaded.value = true
  } catch (err) {
    error.value = err.message || String(err)
  }
})

const pair = computed(() => compareRuns(runs.value, selectedA.value, selectedB.value))

function fmtTime(unix) {
  return new Date(unix * 1000).toLocaleString()
}
</script>

<template>
  <main>
    <RouterLink to="/">{{ t('back.allSwells') }}</RouterLink>
    <h1>{{ t('reports.heading') }}</h1>
    <p v-if="error" class="error" data-test="error">{{ error }}</p>
    <p v-else-if="!loaded" data-test="loading">{{ t('common.loading') }}</p>
    <p v-else-if="runs.length === 0" data-test="empty">
      {{ t('reports.noRuns') }}
    </p>
    <template v-else>
      <section data-test="comparison-section">
        <h2>{{ t('reports.compareTwo') }}</h2>
        <p v-if="runs.length < 2" data-test="empty">
          {{ t('reports.noSecond') }}
        </p>
        <template v-else>
          <p>
            <label>
              {{ t('reports.runA') }}
              <select v-model="selectedA" data-test="select-a">
                <option v-for="r in runs" :key="r.run_id" :value="r.run_id">
                  {{ r.run_id }} ({{ r.swell_id }})
                </option>
              </select>
            </label>
            <label>
              {{ t('reports.runB') }}
              <select v-model="selectedB" data-test="select-b">
                <option v-for="r in runs" :key="r.run_id" :value="r.run_id">
                  {{ r.run_id }} ({{ r.swell_id }})
                </option>
              </select>
            </label>
          </p>
          <table data-test="comparison-table">
            <thead>
              <tr>
                <th>{{ t('reports.colField') }}</th>
                <th data-test="head-a">{{ selectedA }}</th>
                <th data-test="head-b">{{ selectedB }}</th>
              </tr>
            </thead>
            <tbody>
              <tr data-test="comparison-row">
                <th scope="row">{{ t('reports.rowRun') }}</th>
                <td>{{ pair.a ? pair.a.run_id : '—' }}</td>
                <td>{{ pair.b ? pair.b.run_id : '—' }}</td>
              </tr>
              <tr data-test="comparison-row">
                <th scope="row">{{ t('reports.rowSwell') }}</th>
                <td>{{ pair.a ? pair.a.swell_id : '—' }}</td>
                <td>{{ pair.b ? pair.b.swell_id : '—' }}</td>
              </tr>
              <tr data-test="comparison-row">
                <th scope="row">{{ t('reports.rowSpectrum') }}</th>
                <td>{{ pair.a ? pair.a.spectrum_path : '—' }}</td>
                <td>{{ pair.b ? pair.b.spectrum_path : '—' }}</td>
              </tr>
              <tr data-test="comparison-row">
                <th scope="row">{{ t('reports.rowState') }}</th>
                <td>{{ pair.a ? pair.a.state : '—' }}</td>
                <td>{{ pair.b ? pair.b.state : '—' }}</td>
              </tr>
              <tr data-test="comparison-row">
                <th scope="row">{{ t('reports.rowCreated') }}</th>
                <td>{{ pair.a ? fmtTime(pair.a.created_at) : '—' }}</td>
                <td>{{ pair.b ? fmtTime(pair.b.created_at) : '—' }}</td>
              </tr>
            </tbody>
          </table>
        </template>
      </section>
    </template>

    <template v-if="loaded && !error">
      <section data-test="report-structure-section">
        <h2>{{ t('reports.structure') }}</h2>
        <ul>
          <li v-for="s in REPORT_SECTIONS" :key="s.section" data-test="report-section">
            <code>{{ s.section }}</code> — {{ t(s.contentKey) }}
          </li>
        </ul>
      </section>

      <p class="unwired-note" data-test="unwired-note">
        {{ t('reports.unwiredNote') }}
      </p>
    </template>
  </main>
</template>

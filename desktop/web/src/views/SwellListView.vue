<script setup>
import { onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { fetchStatus, deriveSwells, formatStates, stateTone } from '../api.js'
import SeaBand from '../components/SeaBand.vue'

const { t } = useI18n()

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

// Row waveform mark (26x14): crest height encodes this swell's share of
// total runs — the status payload carries no RPS yet, so the mark
// encodes run-count share and the note under the table says so. The
// phase derives deterministically from the swell id.
function sparkPath(swell) {
  const total = swells.value.reduce((sum, s) => sum + s.run_count, 0)
  const share = total ? swell.run_count / total : 0
  const amp = 2 + share * 10
  const phase =
    [...swell.swell_id].reduce((hash, ch) => (hash * 31 + ch.charCodeAt(0)) % 7, 1) % 7
  const points = []
  for (let i = 0; i <= 13; i++) {
    const x = (i / 13) * 26
    const y = 13 - amp * Math.max(0, Math.sin((i / 13) * Math.PI * 3 + phase))
    points.push(`${i ? 'L' : 'M'}${x.toFixed(1)},${y.toFixed(1)}`)
  }
  return points.join('')
}
</script>

<template>
  <main>
    <h1>{{ t('swells.heading') }}</h1>
    <SeaBand v-if="!error" :swells="swells" @open="openSwell" />
    <div v-if="error" class="error error-card" data-test="error">
      <span>{{ error }}</span>
      <button data-test="retry" @click="load">{{ t('common.retry') }}</button>
    </div>
    <p v-else-if="swells === null" data-test="loading">{{ t('common.loading') }}</p>
    <p v-else-if="swells.length === 0" data-test="empty">{{ t('swells.noSwell') }}</p>
    <table v-else data-test="swell-table">
      <thead>
        <tr>
          <th>{{ t('swells.colWave') }}</th>
          <th>{{ t('swells.colSwell') }}</th>
          <th>{{ t('swells.colState') }}</th>
          <th class="num">{{ t('swells.colRuns') }}</th>
          <th class="num">{{ t('swells.colRps') }}</th>
          <th class="num">{{ t('swells.colP99') }}</th>
          <th class="num">{{ t('swells.colErrors') }}</th>
          <th class="num">{{ t('swells.colStarted') }}</th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="swell in swells"
          :key="swell.swell_id"
          data-test="swell-row"
          tabindex="0"
          @click="openSwell(swell.swell_id)"
          @keydown.enter.prevent="openSwell(swell.swell_id)"
        >
          <td>
            <svg
              class="wave-mark"
              width="26"
              height="14"
              viewBox="0 0 26 14"
              role="img"
              :aria-label="t('swells.ariaMark', { id: swell.swell_id })"
            >
              <path :d="sparkPath(swell)" fill="none" stroke="var(--sea-surface)" stroke-width="1.5" />
            </svg>
          </td>
          <td>
            <RouterLink
              :to="`/swell/${encodeURIComponent(swell.swell_id)}`"
              @click.stop
            >
              {{ swell.swell_id }}
            </RouterLink>
          </td>
          <td>
            <span
              v-for="[state, count] in Object.entries(swell.states).sort(([a], [b]) =>
                a.localeCompare(b),
              )"
              :key="state"
              class="pill"
              :class="`pill--${stateTone(state)}`"
            >
              {{ state }} · {{ count }}
            </span>
          </td>
          <td class="num">{{ swell.run_count }}</td>
          <td class="num"><span class="unwired" :title="t('swells.tooltipRps')">—</span></td>
          <td class="num"><span class="unwired" :title="t('swells.tooltipP99')">—</span></td>
          <td class="num"><span class="unwired" :title="t('swells.tooltipErrors')">—</span></td>
          <td class="num">{{ new Date(swell.created_at_min * 1000).toLocaleString() }}</td>
        </tr>
      </tbody>
    </table>
    <p v-if="swells && swells.length > 0" class="unwired-note" data-test="unwired-note">
      {{ t('swells.unwiredNote') }}
    </p>
  </main>
</template>

<style scoped>
.error {
  margin-top: 16px;
}

table {
  margin-top: 20px;
}

.pill {
  margin-right: 4px;
}

.wave-mark {
  display: block;
}

tbody tr {
  cursor: pointer;
}
</style>

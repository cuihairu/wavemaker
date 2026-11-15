<script setup>
// Spectrum editing and validation display (WM-0603) plus the planned
// load timeline (WM-0604). The editor keeps the raw YAML verbatim;
// validation and the timeline go through shell commands that reuse
// wm-core's validator and wm-runtime's ArrivalPlan, so nothing here can
// drift from the engine — and no concurrency curve is fabricated (the
// engine has no active-count estimator yet, §4.4).
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  loadSpectrum,
  saveSpectrum,
  validateSpectrum,
  timelineFor,
  timelinePath,
  strideSamples,
} from '../api.js'

const { t } = useI18n()

const path = ref('')
const content = ref('')
const report = ref(null)
const timeline = ref(null)
const savedMessage = ref('')
const error = ref('')

const W = 240
const H = 100

const ratePath = computed(() =>
  timeline.value ? timelinePath(timeline.value.points, (p) => p.rate_per_second, W, H) : ''
)
const cumulativePath = computed(() =>
  timeline.value
    ? timelinePath(timeline.value.points, (p) => p.expected_arrivals, W, H)
    : ''
)
const samples = computed(() =>
  timeline.value ? strideSamples(timeline.value.points, 10) : []
)
const rateMax = computed(() =>
  timeline.value ? Math.max(...timeline.value.points.map((p) => p.rate_per_second)) : 0
)
const cumulativeMax = computed(() =>
  timeline.value
    ? Math.max(...timeline.value.points.map((p) => p.expected_arrivals))
    : 0
)
// "N errors, M warnings" through the pipe-plural keys (api.js's
// summarizeIssues keeps its English form for the protocol-level test).
const issueSummary = computed(() => {
  if (!report.value || report.value.parse_error) return ''
  const issues = report.value.issues || []
  const errors = issues.filter((i) => i.severity === 'error').length
  const warnings = issues.filter((i) => i.severity === 'warning').length
  return `${t('spectrum.issueErrors', errors, { n: errors })}, ${t('spectrum.issueWarnings', warnings, { n: warnings })}`
})

function fmt(v) {
  return Number(v.toFixed(2)).toString()
}

function reset() {
  report.value = null
  timeline.value = null
  savedMessage.value = ''
  error.value = ''
}

async function onLoad() {
  reset()
  try {
    content.value = await loadSpectrum(path.value)
  } catch (err) {
    error.value = err.message || String(err)
  }
}

async function onValidate() {
  reset()
  try {
    report.value = await validateSpectrum(content.value)
  } catch (err) {
    error.value = err.message || String(err)
  }
}

async function onSave() {
  reset()
  try {
    await saveSpectrum(path.value, content.value)
    savedMessage.value = t('spectrum.saved', { path: path.value })
  } catch (err) {
    error.value = err.message || String(err)
  }
}

async function onTimeline() {
  reset()
  try {
    timeline.value = await timelineFor(content.value)
  } catch (err) {
    error.value = err.message || String(err)
  }
}
</script>

<template>
  <main>
    <RouterLink to="/">{{ t('back.allSwells') }}</RouterLink>
    <h1>{{ t('nav.spectrum') }}</h1>
    <p>
      <input
        v-model="path"
        data-test="path-input"
        :placeholder="t('spectrum.placeholder')"
        size="40"
      />
      <button data-test="load" @click="onLoad">{{ t('spectrum.load') }}</button>
      <button data-test="save" @click="onSave">{{ t('spectrum.save') }}</button>
    </p>
    <p v-if="error" class="error" data-test="error">{{ error }}</p>
    <p v-else-if="savedMessage" data-test="saved">{{ savedMessage }}</p>
    <textarea
      v-model="content"
      data-test="editor"
      rows="14"
      cols="70"
      spellcheck="false"
    ></textarea>
    <p>
      <button data-test="validate" @click="onValidate">{{ t('spectrum.validate') }}</button>
      <button data-test="timeline" @click="onTimeline">{{ t('spectrum.showTimeline') }}</button>
    </p>
    <section v-if="report" data-test="report">
      <p v-if="report.parse_error" class="error" data-test="parse-error">
        {{ t('spectrum.parseError', { message: report.parse_error }) }}
      </p>
      <template v-else>
        <p v-if="report.ok" class="ok" data-test="valid">{{ t('spectrum.valid') }}</p>
        <p data-test="issue-summary">{{ issueSummary }}</p>
        <ul>
          <li v-for="issue in report.issues" :key="issue.code + issue.field" data-test="issue">
            <span :class="`issue-${issue.severity}`" data-test="issue-severity">
              {{ issue.severity }}
            </span>
            <strong>{{ issue.code }}</strong>
            <code>{{ issue.field }}</code> — {{ issue.message }}
          </li>
        </ul>
      </template>
    </section>
    <section v-if="timeline" data-test="timeline-section">
      <h2>{{ t('spectrum.plannedLoad') }}</h2>
      <p data-test="timeline-window">
        {{ t('spectrum.window', { horizon: fmt(timeline.horizon_secs), ramp: fmt(timeline.ramp_duration_secs) }) }}
      </p>
      <p data-test="timeline-note">
        {{ t('spectrum.timelineNote') }}
      </p>
      <figure data-test="rate-chart">
        <figcaption>{{ t('spectrum.figRate', { peak: fmt(rateMax) }) }}</figcaption>
        <svg :viewBox="`0 0 ${W} ${H}`" width="480" height="200" role="img"
             :aria-label="t('spectrum.ariaRate')">
          <line :x1="0" :y1="H" :x2="W" :y2="H" stroke="#d5d3cb" stroke-width="1" />
          <path :d="ratePath" fill="none" stroke="#2a78d6" stroke-width="2" />
          <circle
            v-for="s in samples"
            :key="`r${s.t_secs}`"
            :cx="(s.t_secs / timeline.horizon_secs) * W"
            :cy="rateMax <= 0 ? H : H - (s.rate_per_second / rateMax) * H"
            r="2.5"
            fill="#2a78d6"
          >
            <title>{{ t('spectrum.dotRate', { t: fmt(s.t_secs), rate: fmt(s.rate_per_second) }) }}</title>
          </circle>
        </svg>
      </figure>
      <figure data-test="cumulative-chart">
        <figcaption>{{ t('spectrum.figCumulative', { total: fmt(cumulativeMax) }) }}</figcaption>
        <svg :viewBox="`0 0 ${W} ${H}`" width="480" height="200" role="img"
             :aria-label="t('spectrum.ariaCumulative')">
          <line :x1="0" :y1="H" :x2="W" :y2="H" stroke="#d5d3cb" stroke-width="1" />
          <path :d="cumulativePath" fill="none" stroke="#2a78d6" stroke-width="2" />
          <circle
            v-for="s in samples"
            :key="`c${s.t_secs}`"
            :cx="(s.t_secs / timeline.horizon_secs) * W"
            :cy="cumulativeMax <= 0 ? H : H - (s.expected_arrivals / cumulativeMax) * H"
            r="2.5"
            fill="#2a78d6"
          >
            <title>{{ t('spectrum.dotCumulative', { t: fmt(s.t_secs), total: fmt(s.expected_arrivals) }) }}</title>
          </circle>
        </svg>
      </figure>
      <table data-test="timeline-table">
        <thead>
          <tr>
            <th>{{ t('spectrum.colT') }}</th>
            <th>{{ t('spectrum.colLambda') }}</th>
            <th>{{ t('spectrum.colArrivals') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="s in samples" :key="`t${s.t_secs}`" data-test="timeline-row">
            <td>{{ fmt(s.t_secs) }}</td>
            <td>{{ fmt(s.rate_per_second) }}</td>
            <td>{{ fmt(s.expected_arrivals) }}</td>
          </tr>
        </tbody>
      </table>
      <p class="muted">{{ t('spectrum.strideNote') }}</p>
    </section>
  </main>
</template>

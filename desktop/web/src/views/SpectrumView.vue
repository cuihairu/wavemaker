<script setup>
// Spectrum editing and validation display (WM-0603) plus the planned
// load timeline (WM-0604). The editor keeps the raw YAML verbatim;
// validation and the timeline go through shell commands that reuse
// wm-core's validator and wm-runtime's ArrivalPlan, so nothing here can
// drift from the engine — and no concurrency curve is fabricated (the
// engine has no active-count estimator yet, §4.4).
import { computed, ref } from 'vue'
import {
  loadSpectrum,
  saveSpectrum,
  validateSpectrum,
  summarizeIssues,
  timelineFor,
  timelinePath,
  strideSamples,
} from '../api.js'

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
    savedMessage.value = `saved ${path.value}`
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
    <RouterLink to="/">← All swells</RouterLink>
    <h1>Spectrum</h1>
    <p>
      <input
        v-model="path"
        data-test="path-input"
        placeholder="path to spectrum.yaml"
        size="40"
      />
      <button data-test="load" @click="onLoad">Load</button>
      <button data-test="save" @click="onSave">Save</button>
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
      <button data-test="validate" @click="onValidate">Validate</button>
      <button data-test="timeline" @click="onTimeline">Show timeline</button>
    </p>
    <section v-if="report" data-test="report">
      <p v-if="report.parse_error" class="error" data-test="parse-error">
        cannot parse: {{ report.parse_error }}
      </p>
      <template v-else>
        <p v-if="report.ok" class="ok" data-test="valid">Config is valid.</p>
        <p data-test="issue-summary">{{ summarizeIssues(report.issues) }}</p>
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
      <h2>Planned load</h2>
      <p data-test="timeline-window">
        window 0–{{ fmt(timeline.horizon_secs) }}s (ramp
        {{ fmt(timeline.ramp_duration_secs) }}s + hold)
      </p>
      <p data-test="timeline-note">
        No concurrency curve is shown: the engine has no active-count
        estimator yet, and a naive λ·W line would overstate precision.
      </p>
      <figure data-test="rate-chart">
        <figcaption>Arrival rate λ(t) — per second, peak {{ fmt(rateMax) }}</figcaption>
        <svg :viewBox="`0 0 ${W} ${H}`" width="480" height="200" role="img"
             aria-label="planned arrival rate over time">
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
            <title>t={{ fmt(s.t_secs) }}s, λ={{ fmt(s.rate_per_second) }}/s</title>
          </circle>
        </svg>
      </figure>
      <figure data-test="cumulative-chart">
        <figcaption>Expected arrivals Λ(t) — total {{ fmt(cumulativeMax) }}</figcaption>
        <svg :viewBox="`0 0 ${W} ${H}`" width="480" height="200" role="img"
             aria-label="cumulative expected arrivals over time">
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
            <title>t={{ fmt(s.t_secs) }}s, Λ={{ fmt(s.expected_arrivals) }}</title>
          </circle>
        </svg>
      </figure>
      <table data-test="timeline-table">
        <thead>
          <tr>
            <th>t (s)</th>
            <th>λ (1/s)</th>
            <th>Λ arrivals</th>
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
      <p class="muted">every 10th sample shown</p>
    </section>
  </main>
</template>

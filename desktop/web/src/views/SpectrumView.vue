<script setup>
// Spectrum editing and validation display (WM-0603). The editor keeps
// the raw YAML verbatim; validation reuses the shell's wm-core engine
// (`wm validate`), so parse errors and field violations come from the
// same source as the CLI — never a JS reimplementation.
import { ref } from 'vue'
import { loadSpectrum, saveSpectrum, validateSpectrum, summarizeIssues } from '../api.js'

const path = ref('')
const content = ref('')
const report = ref(null)
const savedMessage = ref('')
const error = ref('')

function reset() {
  report.value = null
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
  </main>
</template>

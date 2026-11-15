<script setup>
// Root shell: minimal header, the routed view, and the session state
// (WM-0607). The session lives here and is provided to the views; the
// authoritative permission checks stay on the Rust side — the header
// only mirrors what the shell reports. The header also carries the
// EN/中文 locale switch (WM-I18N-001).
import { onMounted, provide, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { currentSession, logout } from './api.js'
import { setLocale } from './i18n/index.js'

const { t, locale } = useI18n()
const session = ref(null)

provide('session', session)

onMounted(async () => {
  try {
    session.value = await currentSession()
  } catch {
    session.value = null
  }
})

async function onLogout() {
  try {
    await logout()
  } finally {
    session.value = null
  }
}

function switchTo(lang) {
  setLocale(lang)
}
</script>

<template>
  <div class="app">
    <header class="topnav">
      <span class="brand">Wave<b>maker</b></span>
      <nav>
        <RouterLink to="/">{{ t('nav.swells') }}</RouterLink>
        <RouterLink to="/status">{{ t('nav.status') }}</RouterLink>
        <RouterLink to="/spectrum">{{ t('nav.spectrum') }}</RouterLink>
        <RouterLink to="/metrics">{{ t('nav.metrics') }}</RouterLink>
        <RouterLink to="/reports">{{ t('nav.reports') }}</RouterLink>
        <RouterLink to="/users">{{ t('nav.users') }}</RouterLink>
        <RouterLink to="/audit">{{ t('nav.audit') }}</RouterLink>
      </nav>
      <span class="lang-switch">
        <button
          data-test="lang-en"
          :class="{ on: locale === 'en' }"
          @click="switchTo('en')"
        >
          EN
        </button>
        <button
          data-test="lang-zh"
          :class="{ on: locale === 'zh-CN' }"
          @click="switchTo('zh-CN')"
        >
          中文
        </button>
      </span>
      <span class="session-box" data-test="session-box">
        <template v-if="session">
          <span data-test="header-user">{{ session.username }} ({{ session.role }})</span>
          <button data-test="logout" @click="onLogout">{{ t('shell.logout') }}</button>
        </template>
        <RouterLink v-else to="/login" data-test="login-link">{{ t('shell.login') }}</RouterLink>
      </span>
    </header>
    <RouterView />
  </div>
</template>

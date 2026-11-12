<script setup>
// Root shell: minimal header, the routed view, and the session state
// (WM-0607). The session lives here and is provided to the views; the
// authoritative permission checks stay on the Rust side — the header
// only mirrors what the shell reports.
import { onMounted, provide, ref } from 'vue'
import { currentSession, logout } from './api.js'

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
</script>

<template>
  <div class="app">
    <header>
      <span class="brand">Wavemaker</span>
      <nav>
        <RouterLink to="/">Swells</RouterLink>
        <RouterLink to="/status">Status</RouterLink>
        <RouterLink to="/spectrum">Spectrum</RouterLink>
        <RouterLink to="/metrics">Metrics</RouterLink>
        <RouterLink to="/reports">Reports</RouterLink>
        <RouterLink to="/users">Users</RouterLink>
      </nav>
      <span class="session-box" data-test="session-box">
        <template v-if="session">
          <span data-test="header-user">{{ session.username }} ({{ session.role }})</span>
          <button data-test="logout" @click="onLogout">Log out</button>
        </template>
        <RouterLink v-else to="/login" data-test="login-link">Log in</RouterLink>
      </span>
    </header>
    <RouterView />
  </div>
</template>

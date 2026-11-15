<script setup>
// Login page (WM-0607). Local username/password against the shell's
// users file (ADR-009). Unknown user and wrong password surface as the
// distinct named errors the shell returns — never a generic message.
// On success the session ref provided by App.vue is set and the app
// returns to the swell list.
import { inject, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { login } from '../api.js'

const { t } = useI18n()

// Provided by App.vue — the single session state for the header.
const session = inject('session', ref(null))
const router = useRouter()

const username = ref('')
const password = ref('')
const error = ref('')
const busy = ref(false)

async function onLogin() {
  error.value = ''
  busy.value = true
  try {
    session.value = await login(username.value, password.value)
    password.value = ''
    router.push('/')
  } catch (err) {
    error.value = err.message || String(err)
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <main>
    <RouterLink to="/">{{ t('back.allSwells') }}</RouterLink>
    <h1>{{ t('login.heading') }}</h1>
    <section data-test="login-section">
      <form @submit.prevent="onLogin">
        <p>
          <label>
            {{ t('login.username') }}
            <input v-model="username" data-test="username-input" autocomplete="username" />
          </label>
          <label>
            {{ t('login.password') }}
            <input
              v-model="password"
              data-test="password-input"
              type="password"
              autocomplete="current-password"
            />
          </label>
          <button type="submit" data-test="login" :disabled="busy">{{ t('login.heading') }}</button>
        </p>
      </form>
      <p v-if="error" class="error" data-test="error">{{ error }}</p>
    </section>
    <p class="unwired-note" data-test="local-note">
      {{ t('login.localNote') }}
    </p>
  </main>
</template>

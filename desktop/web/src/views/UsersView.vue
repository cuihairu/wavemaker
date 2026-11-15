<script setup>
// Users & permissions page (WM-0607). The permission matrix is rendered
// verbatim from the shell's engine-side matrix (ADR-009 decision 3);
// user management is admin-only and every denial shows the named reason.
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  permissionMatrix,
  currentSession,
  listUsers,
  createUser,
  deleteUser,
  setUserRole,
} from '../api.js'

const { t } = useI18n()

const matrix = ref([])
const session = ref(null)
const users = ref([])
const error = ref('')
const notice = ref('')
const newUsername = ref('')
const newPassword = ref('')
const newRole = ref('viewer')

async function refresh() {
  error.value = ''
  try {
    matrix.value = await permissionMatrix()
    session.value = await currentSession()
    users.value = await listUsers()
  } catch (err) {
    error.value = err.message || String(err)
  }
}

onMounted(refresh)

async function onCreate() {
  error.value = ''
  notice.value = ''
  try {
    await createUser(newUsername.value, newPassword.value, newRole.value)
    notice.value = t('users.noticeCreated', { name: newUsername.value, role: newRole.value })
    newUsername.value = ''
    newPassword.value = ''
    await refresh()
  } catch (err) {
    error.value = err.message || String(err)
  }
}

async function onRole(username, role) {
  error.value = ''
  notice.value = ''
  try {
    await setUserRole(username, role)
    notice.value = t('users.noticeRole', { name: username, role })
    await refresh()
  } catch (err) {
    error.value = err.message || String(err)
  }
}

async function onDelete(username) {
  error.value = ''
  notice.value = ''
  try {
    await deleteUser(username)
    notice.value = t('users.noticeDeleted', { name: username })
    await refresh()
  } catch (err) {
    error.value = err.message || String(err)
  }
}
</script>

<template>
  <main>
    <RouterLink to="/">{{ t('back.allSwells') }}</RouterLink>
    <h1>{{ t('users.heading') }}</h1>
    <p v-if="error" class="error" data-test="error">{{ error }}</p>
    <p v-else-if="notice" data-test="notice">{{ notice }}</p>

    <section data-test="session-section">
      <h2>{{ t('users.session') }}</h2>
      <i18n-t v-if="session" keypath="users.loggedIn" tag="p" data-test="session-info" scope="global">
        <template #user><strong>{{ session.username }}</strong></template>
        <template #role>{{ session.role }}</template>
      </i18n-t>
      <p v-else data-test="session-none">{{ t('users.notLoggedIn') }}</p>
    </section>

    <section data-test="matrix-section">
      <h2>{{ t('users.matrix') }}</h2>
      <table data-test="matrix-table">
        <thead>
          <tr>
            <th>{{ t('users.colAction') }}</th>
            <th>viewer</th>
            <th>operator</th>
            <th>admin</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="row in matrix" :key="row.action" data-test="matrix-row">
            <td><code>{{ row.action }}</code></td>
            <td>{{ row.viewer ? t('users.allow') : t('users.deny') }}</td>
            <td>{{ row.operator ? t('users.allow') : t('users.deny') }}</td>
            <td>{{ row.admin ? t('users.allow') : t('users.deny') }}</td>
          </tr>
        </tbody>
      </table>
      <p class="unwired-note" data-test="control-plane-note">
        {{ t('users.controlPlaneNote') }}
      </p>
    </section>

    <section v-if="session && session.role === 'admin'" data-test="manage-section">
      <h2>{{ t('users.users') }}</h2>
      <p v-if="users.length === 0" data-test="empty">{{ t('users.noUsers') }}</p>
      <table v-else data-test="user-table">
        <thead>
          <tr>
            <th>{{ t('users.colUsername') }}</th>
            <th>{{ t('users.colRole') }}</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="u in users" :key="u.username" data-test="user-row">
            <td>{{ u.username }}</td>
            <td>
              <select
                :value="u.role"
                :data-test="`role-select-${u.username}`"
                @change="onRole(u.username, $event.target.value)"
              >
                <option value="viewer">viewer</option>
                <option value="operator">operator</option>
                <option value="admin">admin</option>
              </select>
            </td>
            <td>
              <button :data-test="`delete-${u.username}`" @click="onDelete(u.username)">
                {{ t('users.delete') }}
              </button>
            </td>
          </tr>
        </tbody>
      </table>

      <h3>{{ t('users.createHeading') }}</h3>
      <form data-test="create-form" @submit.prevent="onCreate">
        <p>
          <label>
            {{ t('users.username') }}
            <input v-model="newUsername" data-test="new-username" />
          </label>
          <label>
            {{ t('users.password') }}
            <input v-model="newPassword" data-test="new-password" type="password" />
          </label>
          <label>
            {{ t('users.role') }}
            <select v-model="newRole" data-test="new-role">
              <option value="viewer">viewer</option>
              <option value="operator">operator</option>
              <option value="admin">admin</option>
            </select>
          </label>
          <button type="submit" data-test="create">{{ t('users.create') }}</button>
        </p>
      </form>
    </section>
    <p v-else-if="session" class="muted" data-test="manage-denied">
      {{ t('users.manageDenied') }}
    </p>
  </main>
</template>

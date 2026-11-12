<script setup>
// Users & permissions page (WM-0607). The permission matrix is rendered
// verbatim from the shell's engine-side matrix (ADR-009 decision 3);
// user management is admin-only and every denial shows the named reason.
import { onMounted, ref } from 'vue'
import {
  permissionMatrix,
  currentSession,
  listUsers,
  createUser,
  deleteUser,
  setUserRole,
} from '../api.js'

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
    notice.value = `created ${newUsername.value} (${newRole.value})`
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
    notice.value = `${username} is now ${role}`
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
    notice.value = `deleted ${username}`
    await refresh()
  } catch (err) {
    error.value = err.message || String(err)
  }
}
</script>

<template>
  <main>
    <RouterLink to="/">← All swells</RouterLink>
    <h1>Users &amp; permissions</h1>
    <p v-if="error" class="error" data-test="error">{{ error }}</p>
    <p v-else-if="notice" data-test="notice">{{ notice }}</p>

    <section data-test="session-section">
      <h2>Session</h2>
      <p v-if="session" data-test="session-info">
        Logged in as <strong>{{ session.username }}</strong> ({{ session.role }}).
      </p>
      <p v-else data-test="session-none">Not logged in.</p>
    </section>

    <section data-test="matrix-section">
      <h2>Permission matrix</h2>
      <table data-test="matrix-table">
        <thead>
          <tr>
            <th>Action</th>
            <th>viewer</th>
            <th>operator</th>
            <th>admin</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="row in matrix" :key="row.action" data-test="matrix-row">
            <td><code>{{ row.action }}</code></td>
            <td>{{ row.viewer ? 'allow' : 'deny' }}</td>
            <td>{{ row.operator ? 'allow' : 'deny' }}</td>
            <td>{{ row.admin ? 'allow' : 'deny' }}</td>
          </tr>
        </tbody>
      </table>
      <p class="unwired-note" data-test="control-plane-note">
        The permission model is local to the desktop shell; the controller
        REST surface remains single-operator without network
        authentication (ADR-008, ADR-009).
      </p>
    </section>

    <section v-if="session && session.role === 'admin'" data-test="manage-section">
      <h2>Users</h2>
      <p v-if="users.length === 0" data-test="empty">No users registered.</p>
      <table v-else data-test="user-table">
        <thead>
          <tr>
            <th>Username</th>
            <th>Role</th>
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
                Delete
              </button>
            </td>
          </tr>
        </tbody>
      </table>

      <h3>Create user</h3>
      <form data-test="create-form" @submit.prevent="onCreate">
        <p>
          <label>
            Username
            <input v-model="newUsername" data-test="new-username" />
          </label>
          <label>
            Password
            <input v-model="newPassword" data-test="new-password" type="password" />
          </label>
          <label>
            Role
            <select v-model="newRole" data-test="new-role">
              <option value="viewer">viewer</option>
              <option value="operator">operator</option>
              <option value="admin">admin</option>
            </select>
          </label>
          <button type="submit" data-test="create">Create</button>
        </p>
      </form>
    </section>
    <p v-else-if="session" class="muted" data-test="manage-denied">
      User management requires the admin role.
    </p>
  </main>
</template>

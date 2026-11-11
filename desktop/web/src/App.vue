<script setup>
// Root shell: a minimal header and the routed view.
</script>

<template>
  <div class="app">
    <header>
      <span class="brand">Wavemaker</span>
      <nav>
        <RouterLink to="/">Swells</RouterLink>
        <RouterLink to="/status">Status</RouterLink>
        <RouterLink to="/spectrum">Spectrum</RouterLink>
      </nav>
    </header>
    <RouterView />
  </div>
</template>

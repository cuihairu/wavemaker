import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    // Dev-only: forward same-origin /api calls to the controller so a
    // plain `pnpm dev` browser session works (the webview path talks
    // through the Tauri HTTP plugin and never sees CORS).
    proxy: {
      '/api': 'http://127.0.0.1:7600',
    },
  },
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  test: {
    environment: 'jsdom',
    globals: true,
  },
})

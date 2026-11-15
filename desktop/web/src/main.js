import { createApp } from 'vue'
import App from './App.vue'
import { createAppRouter } from './router.js'
import { i18n } from './i18n/index.js'
import './styles/main.css'

createApp(App).use(createAppRouter()).use(i18n).mount('#app')

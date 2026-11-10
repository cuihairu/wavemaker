import { createApp } from 'vue'
import App from './App.vue'
import { createAppRouter } from './router.js'

createApp(App).use(createAppRouter()).mount('#app')

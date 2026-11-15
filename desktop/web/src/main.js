import { createApp } from 'vue'
import App from './App.vue'
import { createAppRouter } from './router.js'
import './styles/main.css'

createApp(App).use(createAppRouter()).mount('#app')

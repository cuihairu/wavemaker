import { createRouter, createWebHistory } from 'vue-router'
import SwellListView from './views/SwellListView.vue'
import SwellDetailView from './views/SwellDetailView.vue'
import StatusView from './views/StatusView.vue'

export const routes = [
  { path: '/', name: 'swell-list', component: SwellListView },
  { path: '/swell/:swellId', name: 'swell-detail', component: SwellDetailView },
  { path: '/status', name: 'status', component: StatusView },
]

export function createAppRouter() {
  return createRouter({
    history: createWebHistory(),
    routes,
  })
}

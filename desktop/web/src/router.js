import { createRouter, createWebHistory } from 'vue-router'
import SwellListView from './views/SwellListView.vue'
import SwellDetailView from './views/SwellDetailView.vue'

export const routes = [
  { path: '/', name: 'swell-list', component: SwellListView },
  { path: '/swell/:swellId', name: 'swell-detail', component: SwellDetailView },
]

export function createAppRouter() {
  return createRouter({
    history: createWebHistory(),
    routes,
  })
}

import { createRouter, createWebHistory } from 'vue-router'
import SwellListView from './views/SwellListView.vue'
import SwellDetailView from './views/SwellDetailView.vue'
import StatusView from './views/StatusView.vue'
import SpectrumView from './views/SpectrumView.vue'
import MetricsView from './views/MetricsView.vue'
import ReportsView from './views/ReportsView.vue'

export const routes = [
  { path: '/', name: 'swell-list', component: SwellListView },
  { path: '/swell/:swellId', name: 'swell-detail', component: SwellDetailView },
  { path: '/status', name: 'status', component: StatusView },
  { path: '/spectrum', name: 'spectrum', component: SpectrumView },
  { path: '/metrics', name: 'metrics', component: MetricsView },
  { path: '/reports', name: 'reports', component: ReportsView },
]

export function createAppRouter() {
  return createRouter({
    history: createWebHistory(),
    routes,
  })
}

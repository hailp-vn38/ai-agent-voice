import { createRouter, createWebHistory, type RouteRecordRaw } from 'vue-router'

import { translate } from '@/composables/useI18n'
import type { MessageKey } from '@/i18n/messages'

const routes: RouteRecordRaw[] = [
  { path: '/', redirect: '/overview' },
  { path: '/dashboard', redirect: '/overview' },
  {
    path: '/overview',
    name: 'overview',
    component: () => import('@/views/OverviewView.vue'),
    meta: { titleKey: 'nav.overview' },
  },
  {
    path: '/devices',
    name: 'devices',
    component: () => import('@/views/DevicesView.vue'),
    meta: { titleKey: 'nav.devices' },
  },
  {
    path: '/agents',
    name: 'agents',
    component: () => import('@/views/AgentsView.vue'),
    meta: { titleKey: 'nav.agents' },
  },
  {
    path: '/agents/:agentId',
    name: 'agent-detail',
    component: () => import('@/pages/agents/AgentDetailPage.vue'),
    meta: { titleKey: 'agents.title' },
  },
  {
    path: '/templates',
    name: 'templates',
    component: () => import('@/pages/templates/TemplatesPage.vue'),
    meta: { titleKey: 'nav.templates' },
  },
  {
    path: '/templates/:templateId',
    name: 'template-detail',
    component: () => import('@/pages/templates/TemplateDetailPage.vue'),
    meta: { titleKey: 'templates.title' },
  },
  {
    path: '/mcp',
    name: 'mcp',
    component: () => import('@/views/McpServersView.vue'),
    meta: { titleKey: 'nav.mcp' },
  },
  {
    path: '/providers',
    name: 'providers',
    component: () => import('@/views/ProvidersView.vue'),
    meta: { titleKey: 'nav.providers' },
  },
  {
    path: '/speakers',
    name: 'speakers',
    component: () => import('@/views/SpeakersView.vue'),
    meta: { titleKey: 'nav.speakers' },
  },
  {
    path: '/speakers/:speakerKey',
    name: 'speaker-detail',
    component: () => import('@/pages/speakers/SpeakerDetailPage.vue'),
    meta: { titleKey: 'speakers.title' },
  },
  {
    path: '/system',
    name: 'system',
    component: () => import('@/views/SystemView.vue'),
    meta: { titleKey: 'nav.system' },
  },
  { path: '/:pathMatch(.*)*', redirect: '/overview' },
]

export const router = createRouter({
  history: createWebHistory(),
  routes,
  scrollBehavior: () => ({ top: 0 }),
})

router.afterEach((to) => {
  const page = translate((to.meta.titleKey as MessageKey | undefined) ?? 'nav.agents')
  document.title = translate('app.documentTitle', { page })
})

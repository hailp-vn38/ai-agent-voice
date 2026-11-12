import { createRouter, createWebHistory, type RouteRecordRaw } from 'vue-router'

import { authApi } from '@/api/auth'
import { setSessionHandler } from '@/api/session'
import { translate } from '@/composables/useI18n'
import type { MessageKey } from '@/i18n/messages'
import { useAuthStore, type LoginStatus } from '@/stores/auth'

import { authRedirect } from './auth-guard'
import { waitForSystemLayout } from './system-scroll'

const routes: RouteRecordRaw[] = [
  {
    path: '/login',
    name: 'login',
    component: () => import('@/pages/auth/LoginPage.vue'),
    meta: { public: true, titleKey: 'auth.loginTitle' },
  },
  {
    path: '/change-password',
    name: 'change-password',
    component: () => import('@/pages/auth/ChangePasswordPage.vue'),
    meta: { public: true, titleKey: 'auth.changePasswordTitle' },
  },
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
    path: '/devices/:deviceId',
    name: 'device-detail',
    component: () => import('@/pages/devices/DeviceDetailPage.vue'),
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
    path: '/knowledge',
    name: 'knowledge',
    component: () => import('@/pages/knowledge/KnowledgePage.vue'),
    meta: { titleKey: 'nav.knowledge' },
  },
  {
    path: '/memory',
    name: 'memory',
    component: () => import('@/pages/memory/MemoryPage.vue'),
    meta: { titleKey: 'nav.memory' },
  },
  {
    path: '/jobs',
    name: 'jobs',
    component: () => import('@/pages/jobs/JobsPage.vue'),
    meta: { titleKey: 'nav.jobs' },
  },
  {
    path: '/jobs/settings',
    name: 'jobs-settings',
    component: () => import('@/pages/background/BackgroundPage.vue'),
    meta: { titleKey: 'nav.background' },
  },
  {
    path: '/jobs/settings/runtime',
    name: 'jobs-settings-runtime',
    component: () => import('@/pages/background/BackgroundRuntimePage.vue'),
    meta: { titleKey: 'jobUi.hubRuntimeTitle' },
  },
  {
    path: '/jobs/settings/providers',
    name: 'jobs-settings-providers',
    component: () => import('@/pages/background/BackgroundProvidersPage.vue'),
    meta: { titleKey: 'jobUi.hubProvidersTitle' },
  },
  {
    path: '/jobs/settings/agents',
    name: 'jobs-settings-agents',
    component: () => import('@/pages/background/BackgroundAgentsPage.vue'),
    meta: { titleKey: 'jobUi.hubAgentsTitle' },
  },
  {
    path: '/jobs/settings/skills',
    name: 'jobs-settings-skills',
    component: () => import('@/pages/background/BackgroundSkillsPage.vue'),
    meta: { titleKey: 'jobUi.hubSkillsTitle' },
  },
  {
    path: '/jobs/settings/skills/new',
    name: 'jobs-settings-skill-create',
    component: () => import('@/pages/background/BackgroundSkillDetailPage.vue'),
    meta: { titleKey: 'jobUi.skillCreateTitle' },
  },
  {
    path: '/jobs/settings/skills/:skillId',
    name: 'jobs-settings-skill',
    component: () => import('@/pages/background/BackgroundSkillDetailPage.vue'),
    meta: { titleKey: 'jobUi.hubSkillsTitle' },
  },
  {
    path: '/jobs/settings/agents/:agentId',
    alias: ['/jobs/settings/:agentId'],
    name: 'jobs-settings-agent',
    component: () => import('@/pages/background/BackgroundAgentDetailPage.vue'),
    meta: { titleKey: 'jobUi.agentModalTitle' },
  },
  {
    path: '/jobs/:jobId',
    name: 'job-detail',
    component: () => import('@/pages/jobs/JobDetailPage.vue'),
    meta: { titleKey: 'jobUi.jobDetail' },
  },
  {
    path: '/artifacts',
    name: 'artifacts',
    component: () => import('@/pages/artifacts/ArtifactsPage.vue'),
    meta: { titleKey: 'nav.artifacts' },
  },
  {
    path: '/mcp',
    name: 'mcp',
    component: () => import('@/views/McpServersView.vue'),
    meta: { titleKey: 'nav.mcp' },
  },
  {
    path: '/mcp/:key',
    name: 'mcp-detail',
    component: () => import('@/pages/mcp/McpServerDetailPage.vue'),
    meta: { titleKey: 'nav.mcp' },
  },
  {
    path: '/providers',
    name: 'providers',
    component: () => import('@/views/ProvidersView.vue'),
    meta: { titleKey: 'nav.providers' },
  },
  {
    path: '/providers/create',
    alias: ['/providers/new'],
    name: 'provider-create',
    component: () => import('@/pages/providers/ProviderCreatePage.vue'),
    meta: { titleKey: 'providers.createTitle' },
  },
  {
    path: '/providers/:key',
    name: 'provider-detail',
    component: () => import('@/pages/providers/ProviderDetailPage.vue'),
    meta: { titleKey: 'nav.providers' },
  },
  {
    path: '/llm-gateway',
    name: 'llm-gateway',
    component: () => import('@/pages/gateway/LlmGatewayPage.vue'),
    meta: { titleKey: 'nav.llmGateway' },
  },
  {
    path: '/llm-gateway/pools',
    name: 'llm-gateway-pools',
    component: () => import('@/pages/gateway/LlmGatewayPoolsPage.vue'),
    meta: { titleKey: 'llmGateway.poolsPage.title' },
  },
  {
    path: '/llm-gateway/pools/:vendor',
    name: 'llm-gateway-pool',
    component: () => import('@/pages/gateway/LlmGatewayPoolDetailPage.vue'),
    meta: { titleKey: 'llmGateway.poolsPage.title' },
  },
  {
    path: '/llm-gateway/accounts',
    name: 'llm-gateway-accounts',
    component: () => import('@/pages/gateway/LlmGatewayAccountsPage.vue'),
    meta: { titleKey: 'llmGateway.accountsPage.title' },
  },
  {
    path: '/llm-gateway/accounts/:accountKey',
    name: 'llm-gateway-account',
    component: () => import('@/pages/gateway/LlmGatewayAccountDetailPage.vue'),
    meta: { titleKey: 'llmGateway.accountsPage.title' },
  },
  {
    path: '/llm-gateway/models',
    name: 'llm-gateway-models',
    component: () => import('@/pages/gateway/LlmGatewayModelsPage.vue'),
    meta: { titleKey: 'llmGateway.modelsPage.title' },
  },
  {
    path: '/llm-gateway/models/new',
    name: 'llm-gateway-model-create',
    component: () => import('@/pages/gateway/LlmGatewayModelEditorPage.vue'),
    meta: { titleKey: 'llmGateway.modelPage.createTitle' },
  },
  {
    path: '/llm-gateway/models/:providerKey',
    name: 'llm-gateway-model',
    component: () => import('@/pages/gateway/LlmGatewayModelEditorPage.vue'),
    meta: { titleKey: 'llmGateway.modelsPage.title' },
  },
  {
    path: '/llm-gateway/quotas',
    name: 'llm-gateway-quotas',
    component: () => import('@/pages/gateway/LlmGatewayQuotasPage.vue'),
    meta: { titleKey: 'llmGateway.quotasPage.title' },
  },
  {
    path: '/llm-gateway/connect',
    name: 'llm-gateway-connect',
    component: () => import('@/pages/gateway/LlmGatewayConnectPage.vue'),
    meta: { titleKey: 'llmGateway.connectPage.title' },
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
  {
    path: '/system/monitor',
    alias: ['/system/resources'],
    name: 'system-monitor',
    component: () => import('@/pages/system/SystemMonitorPage.vue'),
    meta: { titleKey: 'system.hub.monitorTitle' },
  },
  {
    path: '/system/runtime',
    name: 'system-runtime',
    component: () => import('@/pages/system/SystemRuntimePage.vue'),
    meta: { titleKey: 'system.hub.runtimeTitle' },
  },
  {
    path: '/system/assistant',
    name: 'system-assistant',
    component: () => import('@/pages/system/SystemAssistantPage.vue'),
    meta: { titleKey: 'system.hub.assistantTitle' },
  },
  {
    path: '/system/backup',
    name: 'system-backup',
    component: () => import('@/pages/system/SystemBackupPage.vue'),
    meta: { titleKey: 'system.hub.backupTitle' },
  },
  {
    path: '/system/transport',
    name: 'system-transport',
    component: () => import('@/pages/system/SystemTransportPage.vue'),
    meta: { titleKey: 'system.hub.transportTitle' },
  },
  {
    path: '/system/mcp',
    alias: ['/mcp-system'],
    name: 'system-mcp',
    component: () => import('@/pages/system/SystemMcpPage.vue'),
    meta: { titleKey: 'nav.system' },
  },
  {
    path: '/system/notifications',
    alias: ['/notifications'],
    name: 'system-notifications',
    component: () => import('@/pages/notifications/NotificationSettingsPage.vue'),
    meta: { titleKey: 'notifications.settings.title' },
  },
  {
    path: '/system/security',
    name: 'system-security',
    component: () => import('@/pages/system/SystemSecurityPage.vue'),
    meta: { titleKey: 'system.hub.securityTitle' },
  },
  {
    path: '/system/logs',
    alias: ['/logs'],
    name: 'system-logs',
    component: () => import('@/pages/system/SystemLogsPage.vue'),
    meta: { titleKey: 'system.logs.title' },
  },
  { path: '/:pathMatch(.*)*', redirect: '/overview' },
]

// Keep the Systems hub context when its detail pages link back via a new navigation.
let systemScrollPosition = { left: 0, top: 0 }

export const router = createRouter({
  history: createWebHistory(),
  routes,
  async scrollBehavior(to, _from, savedPosition) {
    const position = savedPosition ?? (to.name === 'system' ? systemScrollPosition : { left: 0, top: 0 })
    if (to.name === 'system') {
      await waitForSystemLayout()
      // A slow snapshot must not scroll a page the operator has already left.
      if (router.currentRoute.value.fullPath !== to.fullPath) return false
    }
    return position
  },
})

// Startup reads the login state before any data loads. A failed read is not cached, so the next
// navigation tries again; until then the app behaves as if Password Login were on.
async function currentLoginStatus(): Promise<LoginStatus> {
  const auth = useAuthStore()
  if (auth.loginStatus) return auth.loginStatus
  try {
    const response = await authApi.status()
    const status = { passwordLoginEnabled: response.password_login_enabled, mustChangePassword: response.must_change_password }
    auth.setLoginStatus(status)
    return status
  } catch {
    return { passwordLoginEnabled: true, mustChangePassword: false }
  }
}

router.beforeEach((to, from) => {
  if (from.name === 'system' && to.name !== 'system') {
    systemScrollPosition = { left: window.scrollX, top: window.scrollY }
  }
})

router.beforeEach(async to => {
  const status = await currentLoginStatus()
  return authRedirect(to, { status, token: useAuthStore().adminWebToken })
})

setSessionHandler(event => {
  const current = router.currentRoute.value
  if (event.kind === 'password_change_required') {
    if (current.name !== 'change-password') void router.replace({ name: 'change-password', query: { redirect: current.fullPath } })
    return
  }
  if (event.kind === 'signed_out_by_user') {
    void router.replace({ name: 'login' })
    return
  }
  if (current.meta.public) return
  void router.replace({ name: 'login', query: { redirect: current.fullPath } })
})

router.afterEach((to) => {
  const page = translate((to.meta.titleKey as MessageKey | undefined) ?? 'nav.agents')
  document.title = translate('app.documentTitle', { page })
})

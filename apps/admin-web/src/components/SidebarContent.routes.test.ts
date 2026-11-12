import { mount } from '@vue/test-utils'
import { beforeEach, expect, it } from 'vitest'
import { createMemoryHistory, createRouter } from 'vue-router'
import { setLocale } from '@/composables/useI18n'
import { navigationGroups } from '@/config/navigation'
import SidebarContent from './SidebarContent.vue'

beforeEach(() => setLocale('en'))

async function render(path: string, collapsed: boolean) {
  const component = { template: '<div />' }
  // Keep detail records flat, as in the application, and exercise the real RouterLink.
  const routes = [
    ...navigationGroups.flatMap(group => group.items.map(item => ({ path: item.to, component }))),
    ...['/agents/:id', '/templates/:id', '/devices/:id', '/speakers/:id', '/providers/:id', '/mcp/:id', '/jobs/:id', '/jobs/settings/agents/:id', '/llm-gateway/accounts/:id'].map(path => ({ path, component })),
    { path: '/providers/create', alias: '/providers/new', component },
    { path: '/system/monitor', alias: '/system/resources', component },
    { path: '/system/mcp', alias: '/mcp-system', component },
    { path: '/system/notifications', alias: '/notifications', component },
    { path: '/system/logs', alias: '/logs', component },
    { path: '/agents-archive', component },
  ]
  const router = createRouter({ history: createMemoryHistory(), routes })
  await router.push(path)
  await router.isReady()
  const wrapper = mount(SidebarContent, { props: { collapsed, collapsible: true }, global: {
    plugins: [router], stubs: { ThemeToggle: true, LanguageSwitcher: true, NotificationBell: true },
  } })
  return { wrapper, router }
}

const cases = [
  ['/overview', '/overview'], ['/agents/home', '/agents'], ['/templates/home?tab=prompt#config', '/templates'],
  ['/devices/67%3A28%3A43', '/devices'], ['/speakers/alice', '/speakers'], ['/providers/llm', '/providers'],
  ['/providers/new', '/providers'], ['/mcp/knowledge', '/mcp'], ['/jobs/job-1', '/jobs'],
  ['/jobs/settings/agents/worker', '/jobs'], ['/llm-gateway/accounts/main', '/llm-gateway'],
  ['/system/mcp', '/system'], ['/system/resources', '/system'], ['/mcp-system', '/system'],
  ['/notifications', '/system'], ['/logs', '/system'], ['/knowledge?file=wiki%2Fnote.md', '/knowledge'],
] as const

for (const collapsed of [false, true]) {
  it.each(cases)(`marks the parent of %s when collapsed=${collapsed}`, async (path, expected) => {
    const { wrapper } = await render(path, collapsed)
    try {
      const selected = wrapper.findAll('nav a[aria-current="page"]')
      expect(selected).toHaveLength(1)
      expect(selected[0]!.attributes('href')).toBe(expected)
      expect(selected[0]!.classes()).toContain('!text-studio-violet')
    } finally { wrapper.unmount() }
  })
}

it('updates the parent selection when navigating between detail pages', async () => {
  const { wrapper, router } = await render('/agents/home', false)
  try {
    await router.push('/templates/home')
    expect(wrapper.get('nav a[aria-current="page"]').attributes('href')).toBe('/templates')
    expect(wrapper.get('nav a[href="/agents"]').attributes('aria-current')).toBeUndefined()
    await router.push('/agents-archive')
    expect(wrapper.findAll('nav a[aria-current="page"]')).toHaveLength(0)
    expect(wrapper.get('nav a[href="/agents"]').classes()).not.toContain('!text-studio-violet')
  } finally { wrapper.unmount() }
})

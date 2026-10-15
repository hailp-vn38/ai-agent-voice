import { flushPromises, shallowMount } from '@vue/test-utils'
import { reactive } from 'vue'
import { beforeEach, expect, it, vi } from 'vitest'

import CardListSkeleton from '@/components/admin/CardListSkeleton.vue'
import TemplatesPage from '@/pages/templates/TemplatesPage.vue'
import AgentsView from './AgentsView.vue'
import DevicesView from './DevicesView.vue'
import McpServersView from './McpServersView.vue'
import SpeakersView from './SpeakersView.vue'
import ProvidersView from './ProvidersView.vue'

const list = vi.hoisted(() => vi.fn())
vi.mock('@/api/devices', () => ({ devicesApi: { list } }))
vi.mock('@/api/mcp', () => ({ mcpApi: { list } }))
vi.mock('@/api/speakers', () => ({ speakersApi: { list } }))
vi.mock('vue-router', async (importOriginal) => ({
  ...await importOriginal<typeof import('vue-router')>(),
  useRouter: () => ({ push: vi.fn() }),
  useRoute: () => ({ query: {} }),
}))
const store = reactive({
  loading: true, agents: [], templates: [], providers: [], agentTemplateLinks: [], templateLanguages: [],
  revisions: { agents: {} },
  getAgent: () => undefined,
  getTemplatesForAgent: () => [],
  getTemplateAgentCount: () => 0,
  searchProviders: () => [],
  createProviderFromSetup: vi.fn(),
})
vi.mock('@/stores/admin', () => ({ useAdminStore: () => store }))

beforeEach(() => {
  store.loading = true
  list.mockReset()
})

it.each([
  ['agents', AgentsView],
  ['devices', DevicesView],
  ['templates', TemplatesPage],
  ['mcp', McpServersView],
  ['speakers', SpeakersView],
  ['providers', ProvidersView],
] as const)('shows shimmer while %s loads, then shows the empty state', async (_name, component) => {
  let finish!: (value: { items: never[]; total: number; page_size: number }) => void
  list.mockReturnValue(new Promise((resolve) => { finish = resolve }))
  const wrapper = shallowMount(component, { global: { stubs: { CardListSkeleton: false } } })
  try {
    await flushPromises()
    expect(wrapper.findComponent(CardListSkeleton).exists()).toBe(true)
    expect(wrapper.get('[role="status"]').attributes('aria-busy')).toBe('true')
    expect(wrapper.findAll('.card-shimmer')).toHaveLength(6)
    expect(wrapper.findAll('.card-shimmer').every((card) => card.attributes('aria-hidden') === 'true')).toBe(true)
    expect(wrapper.find('.text-center').exists()).toBe(false)

    store.loading = false
    finish({ items: [], total: 0, page_size: 50 })
    await flushPromises()
    expect(wrapper.findComponent(CardListSkeleton).exists()).toBe(false)
    expect(wrapper.find('.text-center').exists()).toBe(true)
  } finally {
    wrapper.unmount()
  }
})

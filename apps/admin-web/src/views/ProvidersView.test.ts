import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent } from 'vue'
import { createMemoryHistory, createRouter, RouterView } from 'vue-router'
import { expect, it, vi } from 'vitest'

import DetailHeader from '@/components/admin/DetailHeader.vue'
import PageHeader from '@/components/admin/PageHeader.vue'
import ProviderCatalogCard from '@/components/providers/ProviderCatalogCard.vue'
import ProviderCreateDrawer from '@/components/providers/ProviderCreateDrawer.vue'
import { providerAdaptersApi } from '@/api/provider-adapters'
import ProviderDetailPage from '@/pages/providers/ProviderDetailPage.vue'
import ProvidersView from './ProvidersView.vue'

const providers = [
  { id: 'tts_primary', name: 'TTS Primary', type: 'tts', status: 'ready', adapter: 'test', model: '', description: '' },
  { id: 'llm_primary', name: 'LLM Primary', type: 'llm', status: 'ready', adapter: 'test', model: '', description: '' },
]
vi.mock('@/stores/admin', () => ({
  useAdminStore: () => ({
    providers, templates: [],
    searchProviders: () => providers,
    templatesUsingProvider: () => [],
    refreshAll: vi.fn(), createProvider: vi.fn(), createProviderFromSetup: vi.fn(),
  }),
}))
vi.mock('@/api/providers', () => ({
  providersApi: {
    get: vi.fn(async (key: string) => ({
      key, name: 'TTS Primary', type: 'tts', adapter: 'test', config_json: '{}', enabled: 1,
    })),
    templates: vi.fn(async () => ({ items: [], total: 0 })),
  },
}))
vi.mock('@/api/provider-adapters', () => ({
  providerAdaptersApi: { list: vi.fn(async () => []), get: vi.fn(async () => ({ config_schema: { fields: [] } })) },
}))

it.each(['vad', 'asr', 'llm', 'tts'])('opens Create with the currently viewed %s type', async (type) => {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/providers', component: ProvidersView }],
  })
  await router.push(`/providers?type=${type}`)
  await router.isReady()
  const wrapper = mount(ProvidersView, {
    global: {
      plugins: [router],
      stubs: { BaseModal: { template: '<div><slot /></div>' }, ProviderFormModal: true, LinkProviderTemplateDialog: true, ConfirmDialog: true, ProviderTestPanel: true },
    },
  })
  try {
    await wrapper.getComponent(PageHeader).get('button').trigger('click')
    await flushPromises()
    expect(wrapper.getComponent(ProviderCreateDrawer).props('initialType')).toBe(type)
    expect(providerAdaptersApi.list).toHaveBeenLastCalledWith(type)
  } finally {
    wrapper.unmount()
  }
})

it.each(['header', 'browser'])('keeps the TTS tab after opening details and going back via %s', async (back) => {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/providers', name: 'providers', component: ProvidersView },
      { path: '/providers/:key', name: 'provider-detail', component: ProviderDetailPage },
    ],
  })
  await router.push('/providers')
  await router.isReady()
  const wrapper = mount(defineComponent({ components: { RouterView }, template: '<RouterView />' }), {
    global: {
      plugins: [router],
      stubs: { ProviderCreateDrawer: true, ProviderFormModal: true, LinkProviderTemplateDialog: true, ConfirmDialog: true, ProviderTestPanel: true },
    },
  })
  try {
    const ttsTab = wrapper.findAll('[role="tab"]').find((tab) => tab.text().includes('TTS'))!
    await ttsTab.trigger('click')
    await flushPromises()
    expect(wrapper.findAllComponents(ProviderCatalogCard)).toHaveLength(1)
    wrapper.getComponent(ProviderCatalogCard).vm.$emit('open')
    await flushPromises()
    expect(router.currentRoute.value.path).toBe('/providers/tts_primary')
    if (back === 'header') wrapper.getComponent(DetailHeader).vm.$emit('back')
    else router.back()
    await flushPromises()
    expect(router.currentRoute.value.path).toBe('/providers')
    expect(wrapper.get('[role="tab"][aria-selected="true"]').text()).toContain('TTS')
    expect(wrapper.findAllComponents(ProviderCatalogCard).map((card) => card.props('provider').id)).toEqual(['tts_primary'])
  } finally {
    wrapper.unmount()
  }
})

import { shallowMount } from '@vue/test-utils'
import { expect, it, vi } from 'vitest'

import { setLocale } from '@/composables/useI18n'
import type { AgentTemplate, ProviderInstance } from '@/domain/admin'
import VoicePipelineStudio from '@/components/pipeline/VoicePipelineStudio.vue'
import ProviderDetailModal from '@/components/admin/ProviderDetailModal.vue'
import ProviderFormModal from '@/components/admin/ProviderFormModal.vue'
import TemplateConfigurationPanel from '@/components/templates/TemplateConfigurationPanel.vue'
import TemplateDetailPage from './TemplateDetailPage.vue'

const template: AgentTemplate = { id: 'home', name: 'Home', language: 'vi', description: '', prompt: 'Keep it brief.', providerBindings: { llm: 'llm' }, createdAt: '', updatedAt: '' }
const provider: ProviderInstance = { id: 'llm', name: 'Shared LLM', type: 'llm', adapter: 'openai', model: 'chat', description: '', status: 'ready' }
const store = {
  providers: [provider], agents: [],
  getTemplate: () => template,
  getAgentsUsingTemplate: () => [],
  getTemplateAgentCount: () => 0,
  templateProviderCount: () => 1,
  devicesUsingTemplate: () => [],
  templatesUsingProvider: () => [template, { ...template, id: 'office', name: 'Office' }],
  updateTemplate: vi.fn(), updateProvider: vi.fn(),
}
vi.mock('@/stores/admin', () => ({ useAdminStore: () => store }))
vi.mock('vue-router', () => ({ useRoute: () => ({ params: { templateId: 'home' } }), useRouter: () => ({ push: vi.fn() }) }))

it('keeps inline prompt saving connected from the new workspace', () => {
  setLocale('en')
  const wrapper = shallowMount(TemplateDetailPage)
  try {
    expect(wrapper.getComponent(TemplateConfigurationPanel).props('context')).toBe('template')
    wrapper.getComponent(TemplateConfigurationPanel).vm.$emit('savePrompt', 'New instructions')
    expect(store.updateTemplate).toHaveBeenCalledWith('home', { prompt: 'New instructions' })
  } finally { wrapper.unmount() }
})

it('resolves every template sharing an instance and preserves the edit action', async () => {
  const wrapper = shallowMount(TemplateDetailPage)
  try {
    wrapper.getComponent(VoicePipelineStudio).vm.$emit('openProvider', provider)
    await wrapper.vm.$nextTick()
    const detail = wrapper.getComponent(ProviderDetailModal)
    expect(detail.props('modelValue')).toBe(true)
    expect(detail.props('usage')?.map((item) => item.templateId)).toEqual(['home', 'office'])
    detail.vm.$emit('edit', provider)
    await wrapper.vm.$nextTick()
    expect(wrapper.getComponent(ProviderDetailModal).props('modelValue')).toBe(false)
    const edit = wrapper.getComponent(ProviderFormModal)
    expect(edit.props('modelValue')).toBe(true)
    expect(edit.props('provider')).toEqual(provider)
    expect(edit.props('usageCount')).toBe(2)
  } finally { wrapper.unmount() }
})

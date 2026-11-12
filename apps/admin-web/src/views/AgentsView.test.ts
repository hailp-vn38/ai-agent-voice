import { shallowMount } from '@vue/test-utils'
import { expect, it, vi } from 'vitest'

import { setLocale } from '@/composables/useI18n'
import type { Agent } from '@/domain/admin'
import AgentCard from '@/components/admin/AgentCard.vue'
import AgentsView from './AgentsView.vue'

const agents: Agent[] = [
  { id: 'home', name: 'Home assistant', description: 'Living room', defaultTemplateId: '', deviceIds: [], createdAt: '', updatedAt: '' },
  { id: 'cook', name: 'Meal planner', description: 'Kitchen assistant', defaultTemplateId: '', deviceIds: [], createdAt: '', updatedAt: '' },
]
const push = vi.fn()
vi.mock('vue-router', () => ({ useRouter: () => ({ push }) }))
vi.mock('@/stores/admin', () => ({ useAdminStore: () => ({
  agents, templates: [], loading: false, revisions: { agents: {} },
  getDefaultTemplate: () => undefined, getTemplatesForAgent: () => [], devicesForAgent: () => [],
}) }))

it('searches agent purpose and clears the filter while preserving the selected detail route', async () => {
  setLocale('en')
  const wrapper = shallowMount(AgentsView)
  try {
    expect(wrapper.findAllComponents(AgentCard)).toHaveLength(2)
    await wrapper.get('input[type="search"]').setValue('  KITCHEN  ')
    expect(wrapper.findAllComponents(AgentCard)).toHaveLength(1)
    wrapper.getComponent(AgentCard).vm.$emit('open')
    expect(push).toHaveBeenCalledWith('/agents/cook')
    await wrapper.get('button.directory-clear').trigger('click')
    expect(wrapper.findAllComponents(AgentCard)).toHaveLength(2)
    expect(wrapper.find('.directory-empty').exists()).toBe(false)
  } finally { wrapper.unmount() }
})

import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it } from 'vitest'

import { setLocale } from '@/composables/useI18n'
import type { Agent } from '@/domain/admin'
import AgentHeader from './AgentHeader.vue'

const agent: Agent = {
  id: 'home',
  name: 'home',
  description: 'A Vietnamese voice assistant',
  defaultTemplateId: 'test',
  deviceIds: ['1'],
  createdAt: '',
  updatedAt: '',
}

function render(item = agent) {
  return mount(AgentHeader, {
    props: { agent: item, templateCount: 2, deviceCount: 1 },
    global: {
      stubs: {
        AgentActionsMenu: {
          emits: ['deleteAgent'],
          template: '<button data-agent-delete @click="$emit(\'deleteAgent\')">Delete</button>',
        },
      },
    },
  })
}

describe('AgentHeader option A', () => {
  beforeEach(() => setLocale('en'))

  it('renders a compact identity header with the voice Agent icon and counts', () => {
    const wrapper = render()
    expect(wrapper.get('h1').text()).toBe('home')
    expect(wrapper.text()).toContain(agent.description)
    expect(wrapper.get('svg[viewBox="0 0 48 48"]').exists()).toBe(true)
    expect(wrapper.get('dl').text()).toContain('2 templates')
    expect(wrapper.get('dl').text()).toContain('1 device')
  })

  it('preserves all existing Agent Detail actions', async () => {
    const wrapper = render()
    await wrapper.get('[data-agent-back]').trigger('click')
    await wrapper.get('[data-agent-edit]').trigger('click')
    await wrapper.get('[data-agent-add-device]').trigger('click')
    await wrapper.get('[data-agent-delete]').trigger('click')
    for (const action of ['back', 'editAgent', 'addDevice', 'deleteAgent']) {
      expect(wrapper.emitted(action)).toHaveLength(1)
    }
  })

  it('wraps long names and shows a Voice AI fallback when description is empty', () => {
    const wrapper = render({ ...agent, name: 'Long Agent '.repeat(15), description: '' })
    expect(wrapper.get('h1').classes()).toContain('break-words')
    expect(wrapper.text()).toContain('Voice AI Agent')
  })
})

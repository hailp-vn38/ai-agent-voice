import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'

import AgentCard from './AgentCard.vue'
import type { Agent } from '@/domain/admin'

const agent: Agent = {
  id: 'home',
  name: 'Home Assistant',
  description: 'Voice Agent',
  defaultTemplateId: '',
  deviceIds: [],
  createdAt: '',
  updatedAt: '',
}

function renderCard() {
  return mount(AgentCard, {
    props: {
      agent,
      defaultTemplateProviders: [],
      templateCount: 3,
      deviceCount: 2,
    },
  })
}

describe('AgentCard navigation', () => {
  it('uses a single clickable surface over the full card (not only the heading)', async () => {
    const wrapper = renderCard()
    expect(wrapper.get('article').classes()).toContain('cursor-pointer')
    const hitTarget = wrapper.get('[data-agent-card-open]')
    expect(hitTarget.element.tagName).toBe('BUTTON')
    expect(hitTarget.classes()).toContain('absolute')
    expect(hitTarget.classes()).toContain('inset-0')
    await hitTarget.trigger('click')
    expect(wrapper.emitted('open')).toHaveLength(1)
  })

  it('keeps Add Device separate: a click must not also open Agent Detail', async () => {
    const wrapper = renderCard()
    await wrapper.get('[data-agent-card-add-device]').trigger('click')
    expect(wrapper.emitted('addDevice')).toHaveLength(1)
    expect(wrapper.emitted('open')).toBeUndefined()
  })
})

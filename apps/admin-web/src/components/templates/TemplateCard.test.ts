import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import type { AgentTemplate } from '@/domain/admin'
import TemplateCard from './TemplateCard.vue'

const template: AgentTemplate = {
  id: 'default', name: 'Default voice', language: 'vi-VN', description: '', prompt: '',
  providerBindings: {}, createdAt: '', updatedAt: '',
}

describe('TemplateCard', () => {
  it('puts edit in the card actions menu', async () => {
    const wrapper = mount(TemplateCard, {
      props: { template, agentCount: 0, agentNames: [], providerNameById: () => 'Provider' },
      global: {
        stubs: {
          TemplateActionsMenu: {
            props: ['showEdit'],
            emits: ['edit'],
            template: '<button data-template-edit :data-show-edit="showEdit" @click="$emit(\'edit\')" />',
          },
          TemplatePipelineSummary: true,
          Badge: true,
        },
      },
    })

    expect(wrapper.get('[data-template-edit]').attributes('data-show-edit')).toBeDefined()
    await wrapper.get('[data-template-edit]').trigger('click')
    expect(wrapper.emitted('edit')).toEqual([[]])
  })
})

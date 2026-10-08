import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'

import VoicePipelineStrip from './VoicePipelineStrip.vue'
import type { AgentTemplate, ProviderInstance } from '@/domain/admin'
import { setLocale } from '@/composables/useI18n'

const template: AgentTemplate = {
  id: 'home',
  name: 'Home',
  description: '',
  language: 'vi-VN',
  prompt: 'Hello',
  providerBindings: {},
  createdAt: '',
  updatedAt: '',
}

describe('VoicePipelineStrip', () => {
  it('displays server default for every unbound slot, without pretending a provider is ready', () => {
    setLocale('en')
    const wrapper = mount(VoicePipelineStrip, { props: { template, providers: [] } })
    expect(wrapper.text().match(/Server default/g)).toHaveLength(4)
    expect(wrapper.text()).not.toContain('Ready')
  })

  it('shows a bound provider name and leaves unbound roles as server default', () => {
    setLocale('en')
    const provider: ProviderInstance = {
      id: 'my_llm',
      name: 'Home LLM',
      type: 'llm',
      adapter: 'openai',
      model: '',
      description: '',
      status: 'ready',
    }
    const wrapper = mount(VoicePipelineStrip, {
      props: {
        template: { ...template, providerBindings: { llm: provider.id } },
        providers: [provider],
      },
    })
    expect(wrapper.text()).toContain('Home LLM')
    expect(wrapper.text().match(/Server default/g)).toHaveLength(3)
  })
})

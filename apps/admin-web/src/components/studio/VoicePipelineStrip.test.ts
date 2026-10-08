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
  it('shows the provider execution order with three responsive arrows', () => {
    setLocale('vi')
    const wrapper = mount(VoicePipelineStrip, { props: { template, providers: [] } })

    const stages = wrapper.findAll('[data-voice-stage]')
    expect(stages.map((stage) => stage.attributes('data-voice-stage'))).toEqual([
      'vad', 'asr', 'llm', 'tts',
    ])

    const connectors = wrapper.findAll('[data-voice-flow-arrow]')
    expect(connectors).toHaveLength(3)
    for (const connector of connectors) {
      expect(connector.find('[data-direction="horizontal"]').exists()).toBe(true)
      expect(connector.find('[data-direction="vertical"]').exists()).toBe(true)
      expect(connector.attributes('aria-hidden')).toBe('true')
    }
  })

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

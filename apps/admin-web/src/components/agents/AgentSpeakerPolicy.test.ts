import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import AgentSpeakerPolicy from './AgentSpeakerPolicy.vue'
import { ApiError } from '@/api/errors'
import { setLocale } from '@/composables/useI18n'

const agentsApi = vi.hoisted(() => ({
  speakerPolicy: vi.fn(),
  setSpeakerPolicy: vi.fn(),
  agentSpeakers: vi.fn(),
  setAgentSpeaker: vi.fn(),
  unlinkAgentSpeaker: vi.fn(),
}))
const speakersApi = vi.hoisted(() => ({ list: vi.fn() }))
vi.mock('@/api/agents', () => ({ agentsApi }))
vi.mock('@/api/speakers', () => ({ speakersApi }))

const BaseModalStub = {
  props: ['modelValue'],
  template: '<div v-if="modelValue"><slot /><slot name="footer" /></div>',
}
const ButtonStub = {
  props: ['disabled'],
  emits: ['click'],
  template: '<button :disabled="disabled" @click="$emit(\'click\')"><slot /></button>',
}
function mountPolicy() {
  return mount(AgentSpeakerPolicy, {
    props: { agentId: 'agent' },
    global: {
      stubs: { BaseModal: BaseModalStub, Button: ButtonStub, Badge: { template: '<span><slot /></span>' } },
    },
  })
}

describe('AgentSpeakerPolicy: identification only', () => {
  beforeEach(() => {
    setLocale('en')
    vi.resetAllMocks()
    agentsApi.speakerPolicy.mockResolvedValue({
      agent_key: 'agent', mode: 'observe', revision: 1, speaker_change: 'reconnect',
    })
    agentsApi.agentSpeakers.mockResolvedValue({
      items: [{ speaker_key: 'alice', enabled: true, usable: true }],
      page: 1, page_size: 50, total: 1, agent_revision: 7,
    })
    agentsApi.setSpeakerPolicy.mockResolvedValue({
      agent_key: 'agent', mode: 'off', revision: 2, speaker_change: 'reconnect',
    })
    agentsApi.setAgentSpeaker.mockResolvedValue({ agent_revision: 8 })
    agentsApi.unlinkAgentSpeaker.mockResolvedValue(undefined)
    speakersApi.list.mockResolvedValue({
      items: [
        { key: 'alice', name: 'Alice', enabled: true },
        { key: 'bob', name: 'Bob', enabled: true },
      ],
      page: 1, page_size: 100, total: 2,
    })
  })

  it('offers only Off and Observe; no Required gate or calibration', async () => {
    const wrapper = mountPolicy()
    await flushPromises()
    expect(wrapper.find('[data-testid="speaker-policy-mode-off"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="speaker-policy-mode-observe"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="speaker-policy-mode-required"]').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('calibration')
  })

  it('toggles an Agent mode with policy revision CAS', async () => {
    const wrapper = mountPolicy()
    await flushPromises()
    await wrapper.get('[data-testid="speaker-policy-mode-off"]').trigger('click')
    await flushPromises()
    expect(agentsApi.setSpeakerPolicy).toHaveBeenCalledWith('agent', 'off', 1)
  })

  it('binds a Speaker directly without Template picker', async () => {
    const wrapper = mountPolicy()
    await flushPromises()
    await wrapper.get('[data-testid="add-grant"]').trigger('click')
    await wrapper.get('[data-testid="grant-speaker-select"]').setValue('bob')
    await wrapper.get('[data-testid="grant-save"]').trigger('click')
    await flushPromises()
    expect(agentsApi.setAgentSpeaker).toHaveBeenCalledWith('agent', 'bob', 7)
    expect(wrapper.find('input[type="checkbox"]').exists()).toBe(false)
  })

  it('unlinks the Speaker directly', async () => {
    const wrapper = mountPolicy()
    await flushPromises()
    const binding = wrapper.get('[data-testid="speaker-binding-alice"]')
    await binding.get('button').trigger('click')
    await flushPromises()
    expect(agentsApi.unlinkAgentSpeaker).toHaveBeenCalledWith('agent', 'alice', 7)
  })

  it('handles a stale revision with refreshed Agent policy', async () => {
    agentsApi.setSpeakerPolicy.mockRejectedValueOnce(new ApiError('conflict', 409, 'revision_conflict'))
    const wrapper = mountPolicy()
    await flushPromises()
    await wrapper.get('[data-testid="speaker-policy-mode-off"]').trigger('click')
    await flushPromises()
    expect(wrapper.get('[role="alert"]').text()).toContain('Reloaded the latest version')
  })
})

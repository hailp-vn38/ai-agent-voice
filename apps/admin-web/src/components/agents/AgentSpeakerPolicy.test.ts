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
  templates: vi.fn(),
}))
const speakersApi = vi.hoisted(() => ({ list: vi.fn() }))

vi.mock('@/api/agents', () => ({ agentsApi }))
vi.mock('@/api/speakers', () => ({ speakersApi }))

const BaseModalStub = { props: ['modelValue'], template: '<div v-if="modelValue"><slot /></div>' }
const ButtonStub = {
  props: ['disabled'],
  emits: ['click'],
  template: '<button :disabled="disabled" @click="$emit(\'click\')"><slot /></button>',
}

function policy(overrides: Record<string, unknown> = {}) {
  return {
    agent_key: 'agent',
    mode: 'observe',
    revision: 1,
    verification_scope: 'per_utterance',
    speaker_change: 'per_utterance',
    text_turns: 'per_utterance',
    required_available: false,
    required_blockers: ['speaker_calibration_required'],
    ...overrides,
  }
}

function bindings() {
  return {
    items: [
      { speaker_key: 'alice', template_keys: ['first'], enabled: true, usable: true },
      { speaker_key: 'bob', template_keys: [], enabled: false, usable: false },
    ],
    page: 1,
    page_size: 50,
    total: 2,
    agent_revision: 7,
  }
}

function mountComponent() {
  return mount(AgentSpeakerPolicy, {
    props: { agentId: 'agent' },
    global: { stubs: { BaseModal: BaseModalStub, Button: ButtonStub, Badge: { template: '<span><slot /></span>' } } },
  })
}

describe('AgentSpeakerPolicy', () => {
  beforeEach(() => {
    setLocale('en')
    vi.resetAllMocks()
    agentsApi.speakerPolicy.mockResolvedValue(policy())
    agentsApi.agentSpeakers.mockResolvedValue(bindings())
    agentsApi.templates.mockResolvedValue({
      items: [
        { key: 'first', name: 'First', language: 'vi-VN', enabled: true, is_default: true },
        { key: 'next', name: 'Next', language: 'vi-VN', enabled: true, is_default: false },
      ],
      revision: 7,
    })
    agentsApi.setSpeakerPolicy.mockResolvedValue(policy({ mode: 'off', revision: 2 }))
    agentsApi.setAgentSpeaker.mockResolvedValue({ agent_revision: 8 })
    agentsApi.unlinkAgentSpeaker.mockResolvedValue(undefined)
    speakersApi.list.mockResolvedValue({
      items: [{ key: 'alice', name: 'Alice', enabled: true }],
      page: 1,
      page_size: 100,
      total: 1,
    })
  })

  it('disables required mode while the qualification gate is missing', async () => {
    const wrapper = mountComponent()
    await flushPromises()

    const required = wrapper.get('[data-testid="speaker-policy-mode-required"]')
    expect(required.attributes('disabled')).toBeDefined()
    expect(wrapper.get('[data-testid="required-blockers"]').text()).toContain('qualified calibration')
  })

  it('reports revision conflicts from policy updates', async () => {
    const conflict = new ApiError('conflict', 409, 'revision_conflict')
    agentsApi.setSpeakerPolicy.mockRejectedValueOnce(conflict)

    const wrapper = mountComponent()
    await flushPromises()
    await wrapper.get('[data-testid="speaker-policy-mode-off"]').trigger('click')
    await flushPromises()

    expect(agentsApi.setSpeakerPolicy).toHaveBeenCalledWith('agent', 'off', 1)
    expect(wrapper.get('[role="alert"]').text()).toContain('Reloaded the latest version')
  })

  it('saves a grant with the agent revision returned by the binding page', async () => {
    const wrapper = mountComponent()
    await flushPromises()

    await wrapper.get('[data-testid="add-grant"]').trigger('click')
    await wrapper.get('select').setValue('alice')
    await wrapper.findAll('input[type="checkbox"]')[1].setValue(true)
    await wrapper.get('[data-testid="grant-save"]').trigger('click')
    await flushPromises()

    expect(agentsApi.setAgentSpeaker).toHaveBeenCalledWith('agent', 'alice', ['first', 'next'], 7)
  })

  it('requires a template before saving a speaker grant', async () => {
    speakersApi.list.mockResolvedValue({
      items: [{ key: 'bob', name: 'Bob', enabled: true }],
      page: 1,
      page_size: 100,
      total: 1,
    })
    const wrapper = mountComponent()
    await flushPromises()

    await wrapper.get('[data-testid="add-grant"]').trigger('click')
    await wrapper.get('select').setValue('bob')

    expect(wrapper.get('[data-testid="grant-save"]').attributes('disabled')).toBeDefined()
  })

  it('removes a speaker grant when the last template is removed', async () => {
    const wrapper = mountComponent()
    await flushPromises()

    await wrapper.get('[data-testid="speaker-binding-alice"] button[aria-label]').trigger('click')
    await flushPromises()

    expect(agentsApi.unlinkAgentSpeaker).toHaveBeenCalledWith('agent', 'alice', 7)
  })

  it('warns when granted to a disabled speaker', async () => {
    speakersApi.list.mockResolvedValue({
      items: [{ key: 'alice', name: 'Alice', enabled: false }],
      page: 1,
      page_size: 100,
      total: 1,
    })

    const wrapper = mountComponent()
    await flushPromises()

    await wrapper.get('[data-testid="add-grant"]').trigger('click')
    await wrapper.get('select').setValue('alice')
    await flushPromises()

    expect(wrapper.get('[data-testid="grant-speaker-disabled"]').text()).toContain('disabled')
  })
})

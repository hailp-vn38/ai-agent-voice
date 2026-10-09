import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import ProviderFormModal from './ProviderFormModal.vue'
import ProviderTestPanel from '@/components/providers/ProviderTestPanel.vue'

const provider = { id: 'llm_test', name: 'LLM', type: 'llm' as const, adapter: 'openai', model: 'test', description: '', status: 'ready' as const, credential: { id: 'credential', masked_key: 'sk-...7A91', key_version: 1, status: 'active' as const } }
const stubs = { BaseModal: { template: '<div><slot /></div>' }, Button: { template: '<button><slot /></button>' } }

describe('Provider credential replacement', () => {
  it('shows masked metadata, keeps the password empty and only submits a replacement when entered', async () => {
    const wrapper = mount(ProviderFormModal, { props: { modelValue: true, provider }, global: { stubs } })
    expect(wrapper.text()).toContain('sk-...7A91')
    expect(wrapper.get('input[type="password"]').element).toHaveProperty('value', '')
    await wrapper.get('form').trigger('submit')
    expect(wrapper.emitted('save')![0]![0]).not.toHaveProperty('apiKey')
    await wrapper.setProps({ modelValue: false })
    await wrapper.setProps({ modelValue: true })
    await wrapper.get('input[type="password"]').setValue('sk-new-secret-2222')
    await wrapper.get('form').trigger('submit')
    expect(wrapper.emitted('save')![1]![0]).toHaveProperty('apiKey', 'sk-new-secret-2222')
    await wrapper.setProps({ modelValue: false })
    await wrapper.setProps({ modelValue: true })
    expect(wrapper.get('input[type="password"]').element).toHaveProperty('value', '')
  })
})


describe('Provider edit draft credential selection', () => {
  it.each([
    { adapter: 'zipformer', type: 'asr' as const, credentialExpected: false },
    { adapter: 'kokoro', type: 'tts' as const, credentialExpected: false },
    { adapter: 'openai', type: 'llm' as const, credentialExpected: true },
  ])('only reuses a saved credential for credential-bearing $adapter', ({ adapter, type, credentialExpected }) => {
    const wrapper = mount(ProviderFormModal, {
      props: { modelValue: true, provider: { ...provider, adapter, type, desiredRevision: 3, configJson: { model: 'demo' } } },
      global: { stubs: { ...stubs, ProviderTestPanel: true } },
    })
    const draft = wrapper.findComponent(ProviderTestPanel).props('draft')!
    if (credentialExpected) expect(draft.saved_credential).toEqual({ key: provider.id, expected_revision: 3 })
    else expect(draft).not.toHaveProperty('saved_credential')
  })
})

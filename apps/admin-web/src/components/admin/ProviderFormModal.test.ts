import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import ProviderFormModal from './ProviderFormModal.vue'

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

import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ClaimDeviceEnrollmentModal from './ClaimDeviceEnrollmentModal.vue'

const agent = { key: 'may', name: 'Mây', description: null, enabled: true, revision: 1 }
const templates = [
  { key: 'enabled', name: 'Enabled', language: 'vi-VN', enabled: true, is_default: true },
  { key: 'disabled', name: 'Disabled', language: 'vi-VN', enabled: false, is_default: false },
]

function page() {
  return mount(ClaimDeviceEnrollmentModal, {
    props: { modelValue: true, agent, templates },
    global: { stubs: { BaseModal: { template: '<div><slot /></div>' } } },
  })
}

describe('ClaimDeviceEnrollmentModal', () => {
  it('keeps activation code as text, preserves leading zeroes, and omits the Agent-default override', async () => {
    const wrapper = page()
    const code = wrapper.get('input[autocomplete="one-time-code"]')
    expect(code.attributes('type')).toBe('text')
    await code.setValue('000001')
    await wrapper.get('form').trigger('submit')
    expect(wrapper.emitted('submit')).toEqual([[{ code: '000001', agent_key: 'may' }]])
  })

  it('only offers enabled Templates and blocks malformed codes', async () => {
    const wrapper = page()
    expect(wrapper.text()).toContain('Enabled')
    expect(wrapper.text()).not.toContain('Disabled')
    expect(wrapper.get('button[type="submit"]').attributes('disabled')).toBeDefined()
    await wrapper.get('input[autocomplete="one-time-code"]').setValue('123456')
    expect(wrapper.get('button[type="submit"]').attributes('disabled')).toBeUndefined()
  })
})

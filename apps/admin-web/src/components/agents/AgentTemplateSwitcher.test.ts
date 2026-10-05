import { mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'

import AgentTemplateSwitcher from './AgentTemplateSwitcher.vue'

vi.mock('vue-router', () => ({
  useRouter: () => ({ push: vi.fn() }),
}))

const templates = [
  { id: 'default', name: 'Default', description: '', language: 'vi-VN', prompt: '', providerBindings: {}, createdAt: '', updatedAt: '' },
  { id: 'next', name: 'Next', description: '', language: 'vi-VN', prompt: '', providerBindings: {}, createdAt: '', updatedAt: '' },
]

function page(settingDefault = false) {
  return mount(AgentTemplateSwitcher, {
    props: {
      agent: { id: 'agent', name: 'Agent', description: '', defaultTemplateId: 'default', deviceIds: [], createdAt: '', updatedAt: '' },
      templates,
      selectedId: 'next',
      providerCount: () => 0,
      settingDefault,
    },
    global: {
      stubs: {
        ActionMenu: { template: '<div><slot /><slot name="trigger" /></div>' },
        MenuItem: { template: '<button><slot /></button>' },
        Badge: { template: '<span><slot /></span>' },
        Button: { props: ['disabled'], template: '<button :disabled="disabled"><slot /></button>' },
      },
    },
  })
}

describe('AgentTemplateSwitcher', () => {
  it('emits the selected linked template when setting it as default', async () => {
    const wrapper = page()
    const button = wrapper.findAll('button').find((item) => item.text().includes('Set as default'))!

    await button.trigger('click')

    expect(wrapper.emitted('setDefault')).toEqual([['next']])
  })

  it('disables the default action while the server mutation is in flight', () => {
    const wrapper = page(true)
    const button = wrapper.findAll('button').find((item) => item.text().includes('Set as default'))!

    expect(button.attributes('disabled')).toBeDefined()
  })
})

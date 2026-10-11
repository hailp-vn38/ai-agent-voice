import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import type { Agent, AgentTemplate } from '@/domain/admin'
import AgentFormModal from './AgentFormModal.vue'

const template: AgentTemplate = {
  id: 'home', name: 'Home', description: '', language: 'Vietnamese',
  prompt: 'Be brief.', providerBindings: {}, createdAt: '', updatedAt: '',
}

function page(props: Record<string, unknown> = {}) {
  return mount(AgentFormModal, {
    props: { modelValue: true, templates: [template], ...props },
    global: { stubs: { BaseModal: { template: '<div><slot /></div>' } } },
  })
}

describe('AgentFormModal', () => {
  it('creates with an existing template ID and has no template creation fields', async () => {
    const wrapper = page()
    expect(wrapper.findAll('input')).toHaveLength(1)
    expect(wrapper.findAll('textarea')).toHaveLength(1)
    expect(wrapper.findAll('select')).toHaveLength(1)
    await wrapper.get('input').setValue(' Home Assistant ')
    await wrapper.get('textarea').setValue(' Home control ')
    await wrapper.get('select').setValue('home')
    await wrapper.get('form').trigger('submit')
    expect(wrapper.emitted('createWithTemplate')).toEqual([[{
      agent: { name: 'Home Assistant', description: 'Home control' }, templateId: 'home',
    }]])
    expect(wrapper.emitted('update:modelValue')).toEqual([[false]])
  })

  it('requires a template and explains an empty catalog', async () => {
    const wrapper = page({ templates: [] })
    await wrapper.get('input').setValue('Home Assistant')
    expect(wrapper.get('button[type="submit"]').attributes('disabled')).toBeDefined()
    expect(wrapper.text()).toContain('Create a template on the Templates page first.')
    await wrapper.get('form').trigger('submit')
    expect(wrapper.emitted('createWithTemplate')).toBeUndefined()
    expect(wrapper.emitted('update:modelValue')).toBeUndefined()
  })

  it('rejects a selection removed from the catalog before submission', async () => {
    const wrapper = page()
    await wrapper.get('input').setValue('Home Assistant')
    await wrapper.get('select').setValue('home')
    await wrapper.setProps({ templates: [] })
    await wrapper.get('form').trigger('submit')
    expect(wrapper.emitted('createWithTemplate')).toBeUndefined()
  })

  it('keeps identity editing independent of template selection', async () => {
    const agent: Agent = {
      id: 'agent', name: 'Home Assistant', description: '', defaultTemplateId: 'home',
      deviceIds: [], createdAt: '', updatedAt: '',
    }
    const wrapper = page({ agent })
    expect(wrapper.find('select').exists()).toBe(false)
    await wrapper.get('input').setValue('Renamed')
    await wrapper.get('form').trigger('submit')
    expect(wrapper.emitted('save')).toEqual([[{ name: 'Renamed', description: '' }]])
    expect(wrapper.emitted('createWithTemplate')).toBeUndefined()
  })
})

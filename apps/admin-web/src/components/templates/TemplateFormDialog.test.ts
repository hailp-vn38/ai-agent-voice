import { flushPromises, mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'

import TemplateFormDialog from './TemplateFormDialog.vue'

import type { AgentTemplate, ProviderInstance } from '@/domain/admin'

const BaseModalStub = { template: '<div><slot /><slot name="footer" /></div>' }
const ButtonStub = {
  props: ['disabled', 'type', 'variant'],
  template: '<button :disabled="disabled" :type="type || \'button\'"><slot /></button>',
}

const providers: ProviderInstance[] = [
  { id: 'silero', name: 'Silero', type: 'vad', adapter: 'silero_onnx', model: '', description: '', status: 'ready' },
  { id: 'maichi', name: 'Mai Chi', type: 'tts', adapter: 'zerotts_onnx', model: '', description: '', status: 'ready' },
]

const existing: AgentTemplate = {
  id: 'vietnamese_home',
  name: 'Vietnamese Home',
  description: '',
  language: 'Vietnamese',
  prompt: 'Be brief.',
  providerBindings: { tts: 'maichi' },
  createdAt: '',
  updatedAt: '',
}

function page(props: Record<string, unknown> = {}) {
  return mount(TemplateFormDialog, {
    props: { modelValue: true, providers, save: vi.fn().mockResolvedValue(undefined), ...props },
    global: { stubs: { BaseModal: BaseModalStub, Button: ButtonStub } },
  })
}

function button(wrapper: ReturnType<typeof page>, label: string) {
  return wrapper.findAll('button').find((item) => item.text() === label)!
}

describe('TemplateFormDialog', () => {
  it('creates from the name without exposing or submitting a key', async () => {
    const save = vi.fn().mockResolvedValue(undefined)
    const wrapper = page({ save })
    await wrapper.get('input[placeholder="Vietnamese Home"]').setValue('Vietnamese Home')
    expect(wrapper.find('input[placeholder="vietnamese_home"]').exists()).toBe(false)
    expect(button(wrapper, 'Continue').attributes('disabled')).toBeUndefined()
    await button(wrapper, 'Continue').trigger('click')
    await button(wrapper, 'Continue').trigger('click')
    await button(wrapper, 'Create template').trigger('click')
    await flushPromises()
    expect(save).toHaveBeenCalledWith(expect.not.objectContaining({ key: expect.anything() }))
  })

  it('never offers a vision slot, because the API has no vision binding', async () => {
    const wrapper = page()
    await wrapper.get('input[placeholder="Vietnamese Home"]').setValue('Home')
    await button(wrapper, 'Continue').trigger('click')

    const labels = wrapper.findAll('fieldset legend').map((legend) => legend.text())
    expect(labels).toEqual(['Providers'])
    const options = wrapper.findAll('select').flatMap((select) => select.findAll('option').map((o) => o.text()))
    expect(options).not.toContain('Vision')
  })

  it('keeps the existing id internal when editing', async () => {
    const save = vi.fn().mockResolvedValue(undefined)
    const wrapper = page({ template: existing, save })

    expect(wrapper.find('input[placeholder="vietnamese_home"]').exists()).toBe(false)

    await button(wrapper, 'Continue').trigger('click')
    await button(wrapper, 'Continue').trigger('click')
    await button(wrapper, 'Save template').trigger('click')
    await flushPromises()

    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'vietnamese_home' }),
    )
  })

  it('keeps the review step open and shows the failure when the save rejects', async () => {
    const save = vi.fn().mockRejectedValue(new Error('boom'))
    const wrapper = page({ save })

    await wrapper.get('input[placeholder="Vietnamese Home"]').setValue('Home')
    await button(wrapper, 'Continue').trigger('click')
    await button(wrapper, 'Continue').trigger('click')
    await button(wrapper, 'Create template').trigger('click')
    await flushPromises()

    expect(wrapper.get('[role="alert"]').text()).toContain('boom')
    expect(wrapper.emitted('update:modelValue')).toBeUndefined()
  })
})

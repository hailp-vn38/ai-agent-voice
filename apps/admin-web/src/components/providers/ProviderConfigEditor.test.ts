import { mount } from '@vue/test-utils'
import { expect, it } from 'vitest'
import ProviderConfigEditor from './ProviderConfigEditor.vue'

it('readonly disables every control and rejects synthetic change events', async () => {
  const wrapper = mount(ProviderConfigEditor, { props: {
    readonly: true, modelValue: { voice: 'one', streaming: false, count: 1 },
    schema: { fields: [
      { key: 'voice', type: 'string', enum_values: ['one', 'two'] },
      { key: 'streaming', type: 'boolean' },
      { key: 'count', type: 'integer' },
    ] },
  } })
  for (const control of wrapper.findAll('input, select')) {
    expect(control.attributes('disabled')).toBeDefined()
    await control.trigger('input')
    await control.trigger('change')
  }
  expect(wrapper.emitted('update:modelValue')).toBeUndefined()
  await wrapper.setProps({ readonly: false })
  await wrapper.get('select').setValue('two')
  expect(wrapper.emitted('update:modelValue')?.[0]?.[0]).toEqual({ voice: 'two', streaming: false, count: 1 })
})

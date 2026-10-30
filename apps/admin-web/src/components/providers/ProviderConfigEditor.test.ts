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

it('filters voices by selected language and auto-selects a compatible voice when language changes', async () => {
  const wrapper = mount(ProviderConfigEditor, {
    props: {
      modelValue: { language: 'vi-VN', voice: 'vi-VN-HoaiMyNeural', rate_percent: 0, pitch_hz: 0 },
      schema: {
        fields: [
          { key: 'language', type: 'select', enum_source: 'languages' },
          { key: 'voice', type: 'select', enum_source: 'voices' },
        ],
      },
      capabilities: {
        languages: [
          { id: 'vi-VN', name: 'Vietnamese (Vietnam)' },
          { id: 'ja-JP', name: 'Japanese (Japan)' },
        ],
        voices: [
          { id: 'vi-VN-HoaiMyNeural', name: 'vi-VN-HoaiMyNeural (Female)', languages: ['vi-VN'] },
          { id: 'ja-JP-NanamiNeural', name: 'ja-JP-NanamiNeural (Female)', languages: ['ja-JP'] },
        ],
      },
    },
  })
  const selects = wrapper.findAll('select')
  expect(selects[1]!.findAll('option').map((o) => o.attributes('value'))).toEqual(['', 'vi-VN-HoaiMyNeural'])
  await selects[0]!.setValue('ja-JP')
  expect(wrapper.emitted('update:modelValue')?.[0]?.[0]).toEqual({
    language: 'ja-JP',
    voice: 'ja-JP-NanamiNeural',
    rate_percent: 0,
    pitch_hz: 0,
  })
})

it('filters voices by selected model, hides speed_percent on turbo, and resets voice/speed when switching models', async () => {
  const wrapper = mount(ProviderConfigEditor, {
    props: {
      modelValue: { model: 'vieneu_v3_turbo_fp32', voice: 'hai_dang', language: 'vi-VN' },
      schema: {
        fields: [
          { key: 'model', type: 'select', enum_source: 'models' },
          { key: 'voice', type: 'select', enum_source: 'voices' },
          { key: 'speed_percent', type: 'integer' },
        ],
      },
      capabilities: {
        models: [
          { id: 'vieneu_v3_turbo_fp32', name: 'VieNeu v3 Turbo FP32' },
          { id: 'vieneu_v3_nano_fp32', name: 'VieNeu v3 Nano FP32' },
        ],
        voices: [
          { id: 'hai_dang', name: 'Hải Đăng', model: 'vieneu_v3_turbo_fp32', languages: ['vi-VN'] },
          { id: 'adam', name: 'Adam', model: 'vieneu_v3_turbo_fp32', languages: ['vi-VN'] },
          { id: 'minh_quan', name: 'Minh Quân', model: 'vieneu_v3_nano_fp32', languages: ['vi-VN'] },
          { id: 'adam', name: 'Adam', model: 'vieneu_v3_nano_fp32', languages: ['vi-VN'] },
        ],
      },
    },
  })
  const selects = wrapper.findAll('select')
  expect(selects[1]!.findAll('option').map((o) => o.attributes('value'))).toEqual(['', 'hai_dang', 'adam'])
  expect(wrapper.find('input[type="number"]').exists()).toBe(false)

  await selects[0]!.setValue('vieneu_v3_nano_fp32')
  expect(wrapper.emitted('update:modelValue')?.[0]?.[0]).toEqual({
    model: 'vieneu_v3_nano_fp32',
    voice: 'minh_quan',
    language: 'vi-VN',
  })

  await wrapper.setProps({
    modelValue: { model: 'vieneu_v3_nano_fp32', voice: 'minh_quan', language: 'vi-VN', speed_percent: 120 },
  })
  expect(wrapper.find('input[type="number"]').exists()).toBe(true)

  await wrapper.findAll('select')[0]!.setValue('vieneu_v3_turbo_fp32')
  expect(wrapper.emitted('update:modelValue')?.[1]?.[0]).toEqual({
    model: 'vieneu_v3_turbo_fp32',
    voice: 'hai_dang',
    language: 'vi-VN',
  })
})

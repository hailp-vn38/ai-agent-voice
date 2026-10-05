import { flushPromises, mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'

import ProviderCreateDrawer from './ProviderCreateDrawer.vue'

const adaptersApi = vi.hoisted(() => ({ list: vi.fn(), get: vi.fn(), discoverCapabilities: vi.fn() }))

vi.mock('@/api/provider-adapters', () => ({
  providerAdaptersApi: adaptersApi,
}))

const BaseModalStub = { template: '<div><slot /><slot name="footer" /></div>' }
const ButtonStub = { props: ['disabled', 'type'], template: '<button :disabled="disabled" :type="type || \'button\'"><slot /></button>' }

describe('ProviderCreateDrawer', () => {
  it('does not send a client key and emits the key returned by the server', async () => {
    adaptersApi.list.mockResolvedValue([{ adapter: 'zerotts_onnx', type: 'tts', display_name: 'ZeroTTS' }])
    adaptersApi.get.mockResolvedValue({ adapter: 'zerotts_onnx', type: 'tts', config_schema: { fields: [] } })
    const create = vi.fn().mockResolvedValue({ key: 'tts_6eb737d745d74285ab916b723eed3671' })
    const wrapper = mount(ProviderCreateDrawer, {
      props: { modelValue: false, create },
      global: { stubs: { BaseModal: BaseModalStub, Button: ButtonStub } },
    })

    await wrapper.setProps({ modelValue: true })
    await flushPromises()
    await wrapper.find('select').setValue('zerotts_onnx')
    await flushPromises()
    await wrapper.findAll('button').find((button) => button.text() === 'Tiếp tục')!.trigger('click')
    await wrapper.get('input[placeholder="Giọng Mai Chi"]').setValue('Giọng Mai Chi')
    await wrapper.findAll('button').find((button) => button.text() === 'Tiếp tục')!.trigger('click')

    expect(wrapper.find('input[placeholder="tts_maichi"]').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('Server tự tạo')

    await wrapper.findAll('button').find((button) => button.text() === 'Tạo provider')!.trigger('click')
    await flushPromises()

    expect(create).toHaveBeenCalledWith({
      name: 'Giọng Mai Chi',
      type: 'tts',
      adapter: 'zerotts_onnx',
      config_json: {},
      secret_ref: undefined,
    })
    expect(wrapper.emitted('created')).toEqual([['tts_6eb737d745d74285ab916b723eed3671']])
  })

  it('ignores a late adapter catalog response from the previously selected type', async () => {
    let resolveTts!: (items: Array<{ adapter: string; type: 'tts'; display_name: string }>) => void
    adaptersApi.list.mockImplementation((type: string) => type === 'tts'
      ? new Promise((resolve) => { resolveTts = resolve })
      : Promise.resolve([{ adapter: 'whisper', type: 'asr', display_name: 'Whisper' }]))

    const wrapper = mount(ProviderCreateDrawer, {
      props: { modelValue: false, create: vi.fn() },
      global: { stubs: { BaseModal: BaseModalStub, Button: ButtonStub } },
    })
    await wrapper.setProps({ modelValue: true })
    await wrapper.findAll('button').find((button) => button.text() === 'asr')!.trigger('click')
    await flushPromises()
    resolveTts([{ adapter: 'zerotts_onnx', type: 'tts', display_name: 'ZeroTTS' }])
    await flushPromises()

    expect(wrapper.findAll('select option').map((option) => option.attributes('value'))).toEqual(['', 'whisper'])
  })

  it('clears step-two fields and requires a new adapter after returning and changing type', async () => {
    adaptersApi.list.mockImplementation(async (type: string) => type === 'tts'
      ? [{ adapter: 'zerotts_onnx', type: 'tts', display_name: 'ZeroTTS' }]
      : [{ adapter: 'whisper', type: 'asr', display_name: 'Whisper' }])
    adaptersApi.get.mockResolvedValue({ adapter: 'zerotts_onnx', type: 'tts', config_schema: { fields: [] } })

    const wrapper = mount(ProviderCreateDrawer, {
      props: { modelValue: false, create: vi.fn() },
      global: { stubs: { BaseModal: BaseModalStub, Button: ButtonStub } },
    })
    await wrapper.setProps({ modelValue: true })
    await flushPromises()
    await wrapper.find('select').setValue('zerotts_onnx')
    await flushPromises()
    await wrapper.findAll('button').find((button) => button.text() === 'Tiếp tục')!.trigger('click')
    await wrapper.get('input[placeholder="Giọng Mai Chi"]').setValue('Giọng cũ')

    await wrapper.findAll('button').find((button) => button.text() === 'Quay lại')!.trigger('click')

    expect(wrapper.get('select').element).toHaveProperty('value', '')
    expect(wrapper.findAll('button').find((button) => button.text() === 'Tiếp tục')!.attributes('disabled')).toBeDefined()

    await wrapper.findAll('button').find((button) => button.text() === 'asr')!.trigger('click')
    await flushPromises()

    expect(wrapper.get('select').element).toHaveProperty('value', '')
    expect(wrapper.findAll('button').find((button) => button.text() === 'Tiếp tục')!.attributes('disabled')).toBeDefined()

    await wrapper.find('select').setValue('whisper')
    adaptersApi.get.mockResolvedValue({ adapter: 'whisper', type: 'asr', config_schema: { fields: [] } })
    await flushPromises()
    await wrapper.findAll('button').find((button) => button.text() === 'Tiếp tục')!.trigger('click')

    expect((wrapper.get('input[placeholder="Giọng Mai Chi"]').element as HTMLInputElement).value).toBe('')
    expect(wrapper.find('input[placeholder="tts_maichi"]').exists()).toBe(false)
  })
})

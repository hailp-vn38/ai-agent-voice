import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, it, vi } from 'vitest'
import ProviderVoiceDiscovery from './ProviderVoiceDiscovery.vue'
import ProviderConfigEditor from './ProviderConfigEditor.vue'

const api = vi.hoisted(() => ({ discoverCapabilities: vi.fn(), get: vi.fn() }))
vi.mock('@/api/provider-adapters', () => ({ providerAdaptersApi: api }))
vi.mock('@/api/providers', () => ({ providersApi: api }))
const global = { stubs: { Button: { template: '<button><slot /></button>' } } }
beforeEach(() => vi.clearAllMocks())

it('reuses a saved credential without requesting its plaintext', async () => {
  api.get.mockResolvedValue({ key: 'saved', adapter: 'vieneu_http', revision: 3,
    config_json: '{"base_url":"http://vieneu:8000/v1","voice":"Mai Anh"}', credential: { id: 'sealed' } })
  api.discoverCapabilities.mockResolvedValue({ voices: [{ id: 'Trúc Ly', name: 'Trúc Ly' }] })
  const wrapper = mount(ProviderVoiceDiscovery, { props: { savedKey: 'saved' }, global })
  await wrapper.get('button').trigger('click')
  await flushPromises()
  expect(api.discoverCapabilities).toHaveBeenCalledWith('vieneu_http', {
    selection: { base_url: 'http://vieneu:8000/v1', voice: 'Mai Anh' },
    saved_credential: { key: 'saved', expected_revision: 3 },
  }, expect.any(AbortSignal))
  expect(wrapper.emitted('discovered')?.[0]?.[0]).toEqual({ voices: [{ id: 'Trúc Ly', name: 'Trúc Ly' }] })
})

it('ignores a late list when its endpoint changes', async () => {
  let finish!: (value: Record<string, unknown>) => void
  api.discoverCapabilities.mockReturnValue(new Promise((resolve) => { finish = resolve }))
  const draft = { type: 'tts' as const, adapter: 'vieneu_http', config_json: { base_url: 'http://one/v1' } }
  const wrapper = mount(ProviderVoiceDiscovery, { props: { draft }, global })
  await wrapper.get('button').trigger('click')
  await wrapper.setProps({ draft: { ...draft, config_json: { base_url: 'http://two/v1' } } })
  finish({ voices: [{ id: 'old', name: 'Old' }] })
  await flushPromises()
  expect(wrapper.emitted('discovered')).toEqual([[{ voices: [] }]])
})

it('selects discovered voice IDs while preserving an existing custom selection', async () => {
  const wrapper = mount(ProviderConfigEditor, { props: { modelValue: { voice: 'Custom' },
    schema: { fields: [{ key: 'voice', type: 'string', enum_source: 'voices' }] },
    capabilities: { voices: [{ id: 'Trúc Ly', name: 'Trúc Ly' }] } } })
  expect(wrapper.get('select').element.value).toBe('Custom')
  await wrapper.get('select').setValue('Trúc Ly')
  expect(wrapper.emitted('update:modelValue')?.[0]?.[0]).toEqual({ voice: 'Trúc Ly' })
})

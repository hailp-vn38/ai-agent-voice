import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, expect, it, vi } from 'vitest'
import ProviderTestPanel from './ProviderTestPanel.vue'
import * as microphone from '@/composables/useMicrophoneRecorder'
import { ref } from 'vue'
const api = vi.hoisted(() => ({ testDraftLlm: vi.fn(), testLlm: vi.fn(), testTtsAudio: vi.fn(), testAsr: vi.fn(), testDraftAsr: vi.fn() }))
vi.mock('@/api/providers', () => ({ providersApi: api }))
it('tests draft inference without saving and clears the result after an edit', async () => {
  api.testDraftLlm.mockResolvedValue({ result: { text: 'Actual response' }, metrics: { elapsed_ms: 12 } })
  const wrapper = mount(ProviderTestPanel, { props: { type: 'llm', draft: { type: 'llm', adapter: 'openai', config_json: { model: 'one' } } } })
  await wrapper.get('[data-run-test]').trigger('click')
  await flushPromises()
  expect(api.testDraftLlm).toHaveBeenCalledTimes(1)
  expect(wrapper.text()).toContain('Actual response')
  await wrapper.setProps({ draft: { type: 'llm', adapter: 'openai', config_json: { model: 'two' } } })
  expect(wrapper.text()).not.toContain('Actual response')
  wrapper.unmount()
})
it('uses the saved API regardless of the runtime badge', async () => {
  api.testLlm.mockResolvedValue({ result: { text: 'Saved response' }, metrics: { elapsed_ms: 2 } })
  const wrapper = mount(ProviderTestPanel, { props: { type: 'llm', savedKey: 'saved' } })
  await wrapper.get('[data-run-test]').trigger('click')
  await flushPromises()
  expect(api.testLlm).toHaveBeenCalledWith('saved', expect.objectContaining({ input: expect.any(String) }), expect.any(AbortSignal))
  expect(wrapper.text()).toContain('Saved response')
  wrapper.unmount()
})

afterEach(() => vi.unstubAllGlobals())
it('revokes TTS playback URLs when replacing a result and closing the panel', async () => {
  const createObjectURL = vi.fn().mockReturnValueOnce('blob:one').mockReturnValueOnce('blob:two')
  const revokeObjectURL = vi.fn()
  vi.stubGlobal('URL', class extends URL { static createObjectURL = createObjectURL; static revokeObjectURL = revokeObjectURL })
  api.testTtsAudio.mockResolvedValue({ audio: new Blob(['RIFF'], { type: 'audio/wav' }), elapsedMs: 42 })
  const wrapper = mount(ProviderTestPanel, { props: { type: 'tts', savedKey: 'saved' } })
  await wrapper.get('[data-run-test]').trigger('click'); await flushPromises()
  expect(wrapper.get('audio').attributes('src')).toBe('blob:one')
  await wrapper.get('[data-run-test]').trigger('click'); await flushPromises()
  expect(revokeObjectURL).toHaveBeenCalledWith('blob:one')
  wrapper.unmount()
  expect(revokeObjectURL).toHaveBeenCalledWith('blob:two')
})

it('invalidates a successful response when the test text changes', async () => {
  api.testLlm.mockResolvedValue({ result: { text: 'Previous result' }, metrics: { elapsed_ms: 2 } })
  const wrapper = mount(ProviderTestPanel, { props: { type: 'llm', savedKey: 'saved' } })
  await wrapper.get('[data-run-test]').trigger('click'); await flushPromises()
  await wrapper.get('textarea').setValue('New prompt')
  expect(wrapper.text()).not.toContain('Previous result')
  wrapper.unmount()
})
it('uses capability voice/language overrides and clears playback when an override changes', async () => {
  const revokeObjectURL = vi.fn()
  vi.stubGlobal('URL', class extends URL { static createObjectURL = () => 'blob:voice'; static revokeObjectURL = revokeObjectURL })
  api.testTtsAudio.mockResolvedValue({ audio: new Blob(['RIFF'], { type: 'audio/wav' }), elapsedMs: 42 })
  const wrapper = mount(ProviderTestPanel, { props: { type: 'tts', savedKey: 'saved', capabilities: { voices: [{ id: 'voice-one', name: 'Voice one' }], languages: [{ id: 'vi-VN', name: 'Vietnamese' }] } } })
  await wrapper.findAll('select')[0]!.setValue('voice-one')
  await wrapper.findAll('select')[1]!.setValue('vi-VN')
  await wrapper.get('[data-run-test]').trigger('click'); await flushPromises()
  expect(api.testTtsAudio).toHaveBeenLastCalledWith('saved', { text: 'Xin chào', voice: 'voice-one', language: 'vi-VN' }, expect.any(AbortSignal))
  const player = wrapper.get('audio')
  Object.defineProperty(player.element, 'duration', { value: 1.25 })
  await player.trigger('loadedmetadata')
  expect(wrapper.text()).toContain('1.25 s')
  await wrapper.findAll('select')[0]!.setValue('')
  expect(wrapper.find('audio').exists()).toBe(false)
  expect(revokeObjectURL).toHaveBeenCalledWith('blob:voice')
  wrapper.unmount()
})

it('renders ASR language, audio duration and real-time factor from the response', async () => {
  const recording = ref(false)
  const audio = new Blob(['RIFF'], { type: 'audio/wav' })
  const recorder = vi.spyOn(microphone, 'useMicrophoneRecorder').mockReturnValue({
    recording, starting: ref(false), paused: ref(false), level: ref(0), elapsedMs: ref(0), error: ref(''),
    start: vi.fn(async () => { recording.value = true }), stop: vi.fn(async () => { recording.value = false; return audio }), dispose: vi.fn(), togglePause: vi.fn(),
  })
  api.testAsr.mockResolvedValue({ result: { text: 'Transcript', language: 'vi-VN' }, metrics: { elapsed_ms: 250, audio_duration_ms: 1250, rtf: 0.2 } })
  const wrapper = mount(ProviderTestPanel, { props: { type: 'asr', savedKey: 'saved' } })
  await wrapper.get('.voice-dock-action').trigger('click'); await flushPromises()
  await wrapper.get('.voice-dock-action').trigger('click'); await flushPromises()
  await wrapper.get('[data-run-test]').trigger('click'); await flushPromises()
  expect(api.testAsr).toHaveBeenCalledWith('saved', { audio }, expect.any(AbortSignal))
  expect(wrapper.text()).toContain('Transcript')
  expect(wrapper.text()).toContain('vi-VN')
  expect(wrapper.text()).toContain('1.25 s')
  expect(wrapper.text()).toContain('RTF: 0.200')
  wrapper.unmount()
  recorder.mockRestore()
})

it('uses the waveform dock for create-provider draft ASR without saving a provider', async () => {
  const recording = ref(false)
  const elapsedMs = ref(0)
  const level = ref(0)
  const audio = new Blob(['RIFF'], { type: 'audio/wav' })
  const recorder = vi.spyOn(microphone, 'useMicrophoneRecorder').mockReturnValue({
    recording, starting: ref(false), paused: ref(false), level, elapsedMs, error: ref(''),
    start: vi.fn(async () => { recording.value = true }),
    stop: vi.fn(async () => { recording.value = false; return audio }),
    dispose: vi.fn(), togglePause: vi.fn(),
  })
  api.testDraftAsr.mockResolvedValue({ result: { text: 'Xin chào', language: 'vi-VN' }, metrics: { elapsed_ms: 30 } })
  const draft = { type: 'asr' as const, adapter: 'whisper', config_json: { model: 'base' } }
  const wrapper = mount(ProviderTestPanel, { props: { type: 'asr', draft } })

  expect(wrapper.find('[data-voice-recording-dock]').exists()).toBe(true)
  expect(wrapper.get('[data-run-test]').attributes('disabled')).toBeDefined()
  await wrapper.get('.voice-dock-action').trigger('click')
  await flushPromises()
  expect(recorder.mock.results[0]?.type).toBe('return')
  level.value = 0.75
  elapsedMs.value = 1_200
  await flushPromises()
  expect(wrapper.text()).toContain('00:01')
  expect(wrapper.find('.voice-dock-secondary').exists()).toBe(true)
  await wrapper.get('.voice-dock-action').trigger('click')
  await flushPromises()
  await wrapper.get('[data-run-test]').trigger('click')
  await flushPromises()

  expect(api.testDraftAsr).toHaveBeenCalledWith(draft, audio, expect.any(AbortSignal))
  expect(wrapper.text()).toContain('Xin chào')
  wrapper.unmount()
  recorder.mockRestore()
})

it('renders image and video previews when LLM diagnostics return media and clears on edit', async () => {
  api.testDraftLlm.mockResolvedValueOnce({
    result: {
      text: 'Generated image/png (8 bytes)',
      media: { mime_type: 'image/png', data_url: 'data:image/png;base64,UE5HX0RBVEE=' },
    },
    metrics: { elapsed_ms: 120 },
  })
  const wrapper = mount(ProviderTestPanel, {
    props: { type: 'llm', draft: { type: 'llm', adapter: 'openai', config_json: { model: 'gemini-3-pro-image', tags: ['image'] } } },
  })
  await wrapper.get('[data-run-test]').trigger('click')
  await flushPromises()
  const img = wrapper.get('[data-test-media="image"]')
  expect(img.attributes('src')).toBe('data:image/png;base64,UE5HX0RBVEE=')
  expect(wrapper.text()).toContain('Generated image/png (8 bytes)')

  await wrapper.get('textarea').setValue('Another prompt')
  expect(wrapper.find('[data-test-media="image"]').exists()).toBe(false)

  api.testDraftLlm.mockResolvedValueOnce({
    result: {
      text: 'Generated video/mp4 (8 bytes)',
      media: { mime_type: 'video/mp4', data_url: 'data:video/mp4;base64,TVA0X0RBVEE=' },
    },
    metrics: { elapsed_ms: 240 },
  })
  await wrapper.get('[data-run-test]').trigger('click')
  await flushPromises()
  const video = wrapper.get('[data-test-media="video"]')
  expect(video.attributes('src')).toBe('data:video/mp4;base64,TVA0X0RBVEE=')
  wrapper.unmount()
})

it('filters and deduplicates voices by draft or saved model selection without merging models', () => {
  const capabilities = {
    models: [
      { id: 'vieneu_v3_turbo_fp32', name: 'VieNeu v3 Turbo FP32' },
      { id: 'vieneu_v3_nano_fp32', name: 'VieNeu v3 Nano FP32' },
    ],
    voices: [
      { id: 'hai_dang', name: 'Hải Đăng', model: 'vieneu_v3_turbo_fp32' },
      { id: 'adam', name: 'Adam', model: 'vieneu_v3_turbo_fp32' },
      { id: 'minh_quan', name: 'Minh Quân', model: 'vieneu_v3_nano_fp32' },
      { id: 'adam', name: 'Adam', model: 'vieneu_v3_nano_fp32' },
    ],
  }
  const nanoDraft = mount(ProviderTestPanel, {
    props: {
      type: 'tts',
      capabilities,
      draft: { type: 'tts', adapter: 'vieneu', config_json: { model: 'vieneu_v3_nano_fp32' } },
    },
  })
  expect(nanoDraft.findAll('select')[0]!.findAll('option').map((o) => o.attributes('value'))).toEqual([
    '',
    'minh_quan',
    'adam',
  ])
  nanoDraft.unmount()

  const savedNano = mount(ProviderTestPanel, {
    props: {
      type: 'tts',
      savedKey: 'vieneu-nano-saved',
      model: 'vieneu_v3_nano_fp32',
      capabilities,
    },
  })
  expect(savedNano.findAll('select')[0]!.findAll('option').map((o) => o.attributes('value'))).toEqual([
    '',
    'minh_quan',
    'adam',
  ])
  savedNano.unmount()

  const savedWithoutExplicitModel = mount(ProviderTestPanel, {
    props: { type: 'tts', savedKey: 'vieneu-saved', capabilities },
  })
  expect(savedWithoutExplicitModel.findAll('select')[0]!.findAll('option').map((o) => o.attributes('value'))).toEqual([
    '',
    'hai_dang',
    'adam',
  ])
  savedWithoutExplicitModel.unmount()
})

import { flushPromises, mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'
import { ref } from 'vue'

import { speakersApi } from '@/api/speakers'
import { useMicrophoneRecorder } from '@/composables/useMicrophoneRecorder'
import SpeakerEnrollmentWizard from './SpeakerEnrollmentWizard.vue'

vi.mock('@/api/speakers', () => ({ speakersApi: {
  summary: vi.fn(), capture: vi.fn(), createFromCapture: vi.fn(),
} }))
vi.mock('@/composables/useMicrophoneRecorder', () => ({ useMicrophoneRecorder: vi.fn() }))

describe('SpeakerEnrollmentWizard', () => {
  it('records one sample with server limits and creates a speaker from the capture', async () => {
    vi.mocked(speakersApi.summary).mockResolvedValue({
      available: true, embedding_space_id: 'camplus', dimension: 192,
      enrollment: {
        content_type: 'audio/wav', sample_rate: 16000, channels: 1, bits_per_sample: 16,
        min_clip_ms: 6000, max_clip_ms: 9000, min_speech_ms: 3000,
        max_window_ms: 9000, max_body_bytes: 300000,
      },
      limits: { max_speakers: 100, max_candidates_per_agent: 10 },
    })
    const wav = new Blob(['sample'], { type: 'audio/wav' })
    const recorder = {
      recording: ref(false), elapsedMs: ref(0), level: ref(0), error: ref(''),
      start: vi.fn(async () => { recorder.recording.value = true }),
      stop: vi.fn(async () => { recorder.recording.value = false; return wav }),
      dispose: vi.fn(),
    }
    vi.mocked(useMicrophoneRecorder).mockReturnValue(recorder)
    vi.mocked(speakersApi.capture).mockResolvedValue({
      status: 'accepted', capture_id: 'capture_1',
      quality: { duration_ms: 6000, speech_ms: 5000 }, expires_at: 123456,
    })
    const speaker = {
      key: 'speaker_1', name: 'Mai Chi', description: null, enabled: true,
      revision: 1, voiceprints: [], created_at: 1, updated_at: 1,
    }
    vi.mocked(speakersApi.createFromCapture).mockResolvedValue({ speaker })
    const wrapper = mount(SpeakerEnrollmentWizard, {
      props: { open: false },
      global: { stubs: { BaseModal: { template: '<div><slot /><slot name="footer" /></div>' } } },
    })
    await wrapper.setProps({ open: true })
    await flushPromises()

    expect(wrapper.text()).toContain('6–9 giây')
    await wrapper.get('button').trigger('click')
    await flushPromises()
    expect(recorder.start).toHaveBeenCalledWith({ minClipMs: 6000, maxClipMs: 9000 }, expect.any(Function))
    expect(wrapper.get('button').attributes('disabled')).toBeDefined()
    recorder.elapsedMs.value = 6000
    await flushPromises()
    await wrapper.get('button').trigger('click')
    await flushPromises()
    expect(speakersApi.capture).toHaveBeenCalledWith(wav, expect.any(AbortSignal))
    await wrapper.findAll('button').find((button) => button.text() === 'Tiếp tục')!.trigger('click')
    await wrapper.get('input').setValue(' Mai Chi ')
    await wrapper.findAll('button').find((button) => button.text() === 'Lưu mẫu')!.trigger('click')
    await flushPromises()

    expect(speakersApi.createFromCapture).toHaveBeenCalledWith({
      capture_id: 'capture_1', name: 'Mai Chi', description: undefined,
    })
    expect(wrapper.emitted('completed')).toEqual([[speaker]])
    wrapper.unmount()
    expect(recorder.dispose).toHaveBeenCalled()
  })
})

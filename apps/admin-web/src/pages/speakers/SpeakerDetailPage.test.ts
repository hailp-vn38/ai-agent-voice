import { flushPromises, shallowMount } from '@vue/test-utils'
import { setLocale } from '@/composables/useI18n'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { reactive } from 'vue'

import { speakersApi } from '@/api/speakers'
import type { Speaker } from '@/api/types/speakers'
import SpeakerDetailPage from './SpeakerDetailPage.vue'

const route = reactive({
  params: { speakerKey: 'spk_a' },
  query: { edit: undefined as string | undefined },
})

vi.mock('vue-router', () => ({
  useRoute: () => route,
  useRouter: () => ({ push: vi.fn(), replace: vi.fn() }),
}))

vi.mock('@/api/speakers', () => ({
  speakersApi: {
    get: vi.fn(),
    summary: vi.fn(),
    update: vi.fn(),
    remove: vi.fn(),
    purgeVoiceprint: vi.fn(),
  },
}))

function speaker(key: string): Speaker {
  return {
    key,
    name: key,
    description: null,
    enabled: true,
    revision: 1,
    voiceprints: [],
    enrollment_drafts: [],
    created_at: 1_760_000_000,
    updated_at: 1_760_000_000,
  }
}

describe('SpeakerDetailPage', () => {
  beforeEach(() => {
    route.params.speakerKey = 'spk_a'
    route.query.edit = undefined
    setLocale('en')
    vi.clearAllMocks()
    vi.mocked(speakersApi.get).mockImplementation(async (key) => speaker(key))
    vi.mocked(speakersApi.summary).mockResolvedValue({
      available: true,
      embedding_space_id: 'default',
      dimension: 192,
      enrollment: {},
      limits: { max_speakers: 100, max_candidates_per_agent: 10 },
    } as Awaited<ReturnType<typeof speakersApi.summary>>)
  })

  it('loads the speaker from a direct URL, then reloads on a param-only navigation', async () => {
    const wrapper = shallowMount(SpeakerDetailPage, {
      global: { stubs: { RouterLink: true } },
    })

    await flushPromises()
    expect(speakersApi.get).toHaveBeenCalledWith('spk_a', expect.any(AbortSignal))
    expect(wrapper.text()).toContain('spk_a')

    route.params.speakerKey = 'spk_b'
    await flushPromises()
    expect(speakersApi.get).toHaveBeenLastCalledWith('spk_b', expect.any(AbortSignal))
    expect(wrapper.text()).toContain('spk_b')

    wrapper.unmount()
  })

  it('shows the speaker when the non-critical recognition summary is still loading', async () => {
    vi.mocked(speakersApi.summary).mockImplementation(() => new Promise(() => {}))

    const wrapper = shallowMount(SpeakerDetailPage, {
      global: { stubs: { RouterLink: true } },
    })

    await flushPromises()
    expect(wrapper.text()).toContain('spk_a')
    wrapper.unmount()
  })

  it('opens the edit dialog from the card query and reloads full data after PATCH', async () => {
    route.query.edit = '1'
    const wrapper = shallowMount(SpeakerDetailPage, {
      global: {
        stubs: {
          RouterLink: true,
          BaseModal: {
            props: ['modelValue'],
            template: '<div v-if="modelValue"><slot /><slot name="footer" /></div>',
          },
        },
      },
    })

    await flushPromises()
    expect(wrapper.find('#speaker-edit-form').exists()).toBe(true)
    vi.mocked(speakersApi.update).mockResolvedValue(speaker('spk_a'))
    await wrapper.get('#speaker-edit-form').trigger('submit')
    await flushPromises()
    expect(speakersApi.update).toHaveBeenCalledTimes(1)
    // PATCH responds with empty voiceprint/draft projections: the UI must GET again.
    expect(speakersApi.get).toHaveBeenCalledTimes(2)
    wrapper.unmount()
  })
})

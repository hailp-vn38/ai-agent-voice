import { mount, RouterLinkStub } from '@vue/test-utils'
import { beforeEach, describe, expect, it } from 'vitest'

import type { SpeakerSummary } from '@/api/types/speakers'
import { setLocale } from '@/composables/useI18n'
import SpeakerCard from './SpeakerCard.vue'

const speaker: SpeakerSummary = {
  key: 'spk_f8fbd35de4fe48ba8b949b8481edc9db',
  name: 'Hải',
  description: 'Tên Hải, tuổi 29',
  enabled: true,
  revision: 2,
  updated_at: 1_760_000_000,
}

function renderCard(item: SpeakerSummary = speaker) {
  return mount(SpeakerCard, {
    props: { speaker: item },
    global: { stubs: { RouterLink: RouterLinkStub } },
  })
}

describe('SpeakerCard', () => {
  beforeEach(() => setLocale('en'))
  it('makes the entire card open the detail route without displaying the technical key', () => {
    const wrapper = renderCard()

    const link = wrapper.get('[data-speaker-open]')
    expect(link.attributes('aria-label')).toContain(speaker.name)
    expect(wrapper.findComponent(RouterLinkStub).props('to')).toEqual({
      name: 'speaker-detail',
      params: { speakerKey: speaker.key },
    })
    expect(wrapper.get('[data-speaker-card]').classes()).toContain('group')
    expect(wrapper.text()).toContain(speaker.name)
    expect(wrapper.text()).not.toContain(speaker.key)
  })

  it('keeps edit and delete as independent actions instead of nested links', async () => {
    const wrapper = renderCard()

    expect(wrapper.get('[data-speaker-open]').find('button').exists()).toBe(false)
    await wrapper.get('[data-speaker-edit]').trigger('click')
    expect(wrapper.emitted('edit')).toEqual([[speaker.key]])
    expect(wrapper.emitted('delete')).toBeUndefined()

    await wrapper.get('[data-speaker-delete]').trigger('click')
    expect(wrapper.emitted('delete')).toEqual([[speaker]])
  })

  it('shows the actual enabled status, not an invented enrollment status', () => {
    const wrapper = renderCard({ ...speaker, enabled: false, description: null })
    expect(wrapper.text()).toContain('Disabled')
    expect(wrapper.text()).not.toContain('Enrolled')
  })
})

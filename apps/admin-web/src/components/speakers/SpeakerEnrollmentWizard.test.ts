import { mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'

import SpeakerEnrollmentWizard from './SpeakerEnrollmentWizard.vue'

vi.mock('@/api/providers', () => ({ providersApi: { list: vi.fn() } }))
vi.mock('@/api/speakers', () => ({ speakersApi: { summary: vi.fn() } }))

describe('SpeakerEnrollmentWizard', () => {
  it('defaults a new speaker to validated enrollment while keeping Quick explicit', () => {
    const wrapper = mount(SpeakerEnrollmentWizard, {
      props: { open: true },
      global: { stubs: { BaseModal: { template: '<div><slot /><slot name="footer" /></div>' } } },
    })

    expect(wrapper.get<HTMLInputElement>('input[value="validated"]').element.checked).toBe(true)
    expect(wrapper.text()).toContain('Quick enrollment')
  })
})

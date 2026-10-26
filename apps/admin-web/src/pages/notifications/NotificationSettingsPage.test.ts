import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { resetDemoNotificationsForTest } from '@/api/notifications'
import { setLocale } from '@/composables/useI18n'
import NotificationSettingsPage from './NotificationSettingsPage.vue'

const { navigateMock } = vi.hoisted(() => ({
  navigateMock: vi.fn(),
}))

vi.mock('vue-router', () => ({
  useRouter: () => ({ push: navigateMock }),
}))

describe('NotificationSettingsPage.vue', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    setLocale('en')
    resetDemoNotificationsForTest()
    vi.stubGlobal(
      'fetch',
      vi.fn().mockRejectedValue(new Error('Server notifications offline in test')),
    )
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('renders dedicated notification settings page and navigates back to system view', async () => {
    const wrapper = mount(NotificationSettingsPage)
    await flushPromises()

    expect(wrapper.find('[data-testid="notification-settings-page"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="notification-settings-card"]').exists()).toBe(true)

    // First button in DetailHeader is the back button to System
    await wrapper.find('button').trigger('click')
    expect(navigateMock).toHaveBeenCalledWith({ name: 'system' })

    wrapper.unmount()
  })
})

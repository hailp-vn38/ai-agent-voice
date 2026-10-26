import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { resetDemoNotificationsForTest } from '@/api/notifications'
import { setLocale } from '@/composables/useI18n'
import NotificationBell from './NotificationBell.vue'

const pushMock = vi.fn()

describe('NotificationBell.vue', () => {
  beforeEach(() => {
    setLocale('en')
    pushMock.mockReset()
    resetDemoNotificationsForTest()
    vi.stubGlobal(
      'fetch',
      vi.fn().mockRejectedValue(new Error('Server notifications not deployed yet')),
    )
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('displays unread badge count in normal and compact modes and filters unread items', async () => {
    const wrapper = mount(NotificationBell, {
      props: { compact: true, placement: 'sidebar' },
    })
    await flushPromises()

    const badge = wrapper.get('[data-testid="notification-unread-badge"]')
    expect(Number(badge.text())).toBe(3)

    // Open popover
    await wrapper.get('[data-testid="notification-bell-trigger"]').trigger('click')
    await flushPromises()

    expect(wrapper.find('[data-testid="notification-popover"]').exists()).toBe(true)
    const allItems = wrapper.findAll('[data-testid="notification-item"]')
    expect(allItems.length).toBe(6)

    // Switch to Unread tab
    await wrapper.get('[data-testid="notification-tab-unread"]').trigger('click')
    await flushPromises()

    const unreadItems = wrapper.findAll('[data-testid="notification-item"]')
    expect(unreadItems.length).toBe(3)
    wrapper.unmount()
  })

  it('marks an item read and navigates when clicked, and supports mark-all-read and purge', async () => {
    const wrapper = mount(NotificationBell, {
      props: { compact: false, placement: 'header', onNavigate: pushMock },
    })
    await flushPromises()

    await wrapper.get('[data-testid="notification-bell-trigger"]').trigger('click')
    await flushPromises()

    // Click first unread notification resource button
    const openButtons = wrapper.findAll('[data-testid="notification-open-resource"]')
    expect(openButtons.length).toBeGreaterThan(0)
    await openButtons[0]!.trigger('click')
    await flushPromises()

    expect(pushMock).toHaveBeenCalledWith('/jobs/job-demo-pricing-01')

    // Reopen popover to verify unread count decreased from 3 to 2
    await wrapper.get('[data-testid="notification-bell-trigger"]').trigger('click')
    await flushPromises()
    expect(wrapper.get('[data-testid="notification-unread-badge"]').text()).toBe('2')

    // Mark all read
    await wrapper.get('[data-testid="notification-mark-all-read"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-testid="notification-unread-badge"]').exists()).toBe(false)

    // Purge read items
    await wrapper.get('[data-testid="notification-purge-read"]').trigger('click')
    await flushPromises()
    expect(wrapper.findAll('[data-testid="notification-item"]')).toHaveLength(0)
    wrapper.unmount()
  })

  it('simulates a live demo notification and renders a toast popup', async () => {
    const wrapper = mount(NotificationBell, {
      props: { compact: false, placement: 'header' },
    })
    await flushPromises()

    await wrapper.get('[data-testid="notification-bell-trigger"]').trigger('click')
    await flushPromises()

    await wrapper.get('[data-testid="notification-simulate-demo"]').trigger('click')
    await flushPromises()

    expect(wrapper.get('[data-testid="notification-unread-badge"]').text()).toBe('4')
    const toast = wrapper.find('[data-testid="notification-toast"]')
    expect(toast.exists()).toBe(true)
    wrapper.unmount()
  })
})

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
    expect(toast.classes()).toContain('notification-toast')
    expect(toast.classes()).toContain('top-4')
    expect(toast.classes()).toContain('right-4')
    expect(toast.classes()).not.toContain('bottom-4')
    wrapper.unmount()
  })

  it('applies floating-chat style open/close transition classes and aria-expanded', async () => {
    const wrapper = mount(NotificationBell, {
      props: { compact: true, placement: 'sidebar' },
      global: { stubs: { Transition: false } },
    })
    await flushPromises()

    const trigger = wrapper.get('[data-testid="notification-bell-trigger"]')
    expect(trigger.attributes('aria-expanded')).toBe('false')

    await trigger.trigger('click')
    await flushPromises()

    expect(trigger.attributes('aria-expanded')).toBe('true')
    const popover = wrapper.get('[data-testid="notification-popover"]')
    expect(popover.classes()).toContain('notification-popover')
    expect(popover.classes()).toContain('is-sidebar')

    await trigger.trigger('click')
    await flushPromises()

    expect(trigger.attributes('aria-expanded')).toBe('false')
    wrapper.unmount()
  })

  it('connects to live server SSE stream and renders toast when server emits notification_created', async () => {
    const serverItem = {
      id: 'srv-notif-1',
      topic: 'reminder_due',
      severity: 'warning',
      title: 'Server Reminder Due',
      body: 'Live reminder pushed from Rust NotificationHub SSE.',
      source_kind: 'schedule',
      source_id: 'rem-srv-1',
      metadata: { resource_href: '/devices' },
      is_read: false,
      read_at_ms: null,
      created_at_ms: 1792980000000,
      deliveries: [],
    }
    const encoder = new TextEncoder()
    const ssePayload =
      `event: ready\ndata: {}\n\n` +
      `event: notification_created\ndata: ${JSON.stringify(serverItem)}\n\n`

    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes('/api/admin/notifications/stream')) {
        const stream = new ReadableStream<Uint8Array>({
          start(controller) {
            controller.enqueue(encoder.encode(ssePayload))
            controller.close()
          },
        })
        return new Response(stream, {
          status: 200,
          headers: { 'Content-Type': 'text/event-stream' },
        })
      }
      if (url.includes('/api/admin/notifications/settings')) {
        return new Response(
          JSON.stringify({
            revision: 1,
            web_ui: { enabled: true, toast_enabled: true, retention_days: 30 },
            telegram: {
              enabled: false,
              chat_id: '',
              thread_id: null,
              subscribed_topics: [],
              has_saved_token: false,
              has_env_fallback: false,
              credential_metadata: null,
            },
            zalo: {
              enabled: false,
              recipient_id: '',
              subscribed_topics: [],
              has_saved_token: false,
              has_env_fallback: false,
              credential_metadata: null,
            },
          }),
          { status: 200, headers: { 'Content-Type': 'application/json' } },
        )
      }
      return new Response(
        JSON.stringify({
          items: [serverItem],
          total: 1,
          unread_count: 1,
          page: 1,
          page_size: 30,
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      )
    })
    vi.stubGlobal('fetch', fetchMock)

    const wrapper = mount(NotificationBell, {
      props: { compact: false, placement: 'header' },
    })
    await flushPromises()
    await flushPromises()

    expect(wrapper.get('[data-testid="notification-unread-badge"]').text()).toBe('1')
    const toast = wrapper.find('[data-testid="notification-toast"]')
    expect(toast.exists()).toBe(true)
    expect(toast.text()).toContain('Server Reminder Due')
    wrapper.unmount()
  })

  it('shares store state across sidebar and header bells, fetches settings only once, and hands off toast host on unmount', async () => {
    const serverItem = {
      id: 'srv-notif-shared',
      topic: 'reminder_due',
      severity: 'warning',
      title: 'Shared Reminder',
      body: 'Delivered to shared store',
      source_kind: 'schedule',
      source_id: 'rem-shared',
      metadata: {},
      is_read: false,
      read_at_ms: null,
      created_at_ms: 1792980000000,
      deliveries: [],
    }
    const encoder = new TextEncoder()
    const ssePayload =
      `event: ready\ndata: {}\n\n` +
      `event: notification_created\ndata: ${JSON.stringify(serverItem)}\n\n`

    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes('/api/admin/notifications/stream')) {
        const stream = new ReadableStream<Uint8Array>({
          start(controller) {
            controller.enqueue(encoder.encode(ssePayload))
            controller.close()
          },
        })
        return new Response(stream, {
          status: 200,
          headers: { 'Content-Type': 'text/event-stream' },
        })
      }
      if (url.includes('/api/admin/notifications/settings')) {
        return new Response(
          JSON.stringify({
            revision: 1,
            web_ui: { enabled: true, toast_enabled: true, retention_days: 30 },
            telegram: {
              enabled: false,
              chat_id: '',
              thread_id: null,
              subscribed_topics: [],
              has_saved_token: false,
              has_env_fallback: false,
              credential_metadata: null,
            },
            zalo: {
              enabled: false,
              recipient_id: '',
              subscribed_topics: [],
              has_saved_token: false,
              has_env_fallback: false,
              credential_metadata: null,
            },
          }),
          { status: 200, headers: { 'Content-Type': 'application/json' } },
        )
      }
      return new Response(
        JSON.stringify({
          items: [serverItem],
          total: 1,
          unread_count: 1,
          page: 1,
          page_size: 30,
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      )
    })
    vi.stubGlobal('fetch', fetchMock)

    const sidebarBell = mount(NotificationBell, {
      props: { compact: true, placement: 'sidebar' },
    })
    const headerBell = mount(NotificationBell, {
      props: { compact: true, placement: 'header' },
    })
    await flushPromises()
    await flushPromises()

    const calledUrls = fetchMock.mock.calls.map(call => String(call[0]))
    expect(calledUrls.filter(url => url.includes('/api/admin/notifications/settings'))).toHaveLength(1)
    expect(calledUrls.filter(url => url.includes('/api/admin/notifications/stream'))).toHaveLength(1)

    expect(sidebarBell.get('[data-testid="notification-unread-badge"]').text()).toBe('1')
    expect(headerBell.get('[data-testid="notification-unread-badge"]').text()).toBe('1')

    // Only the first mounted bell renders the toast initially
    expect(sidebarBell.find('[data-testid="notification-toast"]').exists()).toBe(true)
    expect(headerBell.find('[data-testid="notification-toast"]').exists()).toBe(false)

    // Unmounting the sidebar bell transfers toast host responsibility to the header bell
    sidebarBell.unmount()
    await flushPromises()
    expect(headerBell.find('[data-testid="notification-toast"]').exists()).toBe(true)

    headerBell.unmount()
  })
})

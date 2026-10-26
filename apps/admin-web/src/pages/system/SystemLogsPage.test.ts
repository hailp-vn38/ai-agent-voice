import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { setLocale } from '@/composables/useI18n'
import SystemLogsPage from './SystemLogsPage.vue'

describe('SystemLogsPage.vue', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    setLocale('en')
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(new Response('not found', { status: 404 })),
    )
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('loads demo logs, supports pause/resume stream buffering with floating banner, correlation filter, and audit tab', async () => {
    const wrapper = mount(SystemLogsPage, {
      global: {
        stubs: {
          RouterLink: {
            template: '<a><slot /></a>',
          },
        },
      },
    })
    await flushPromises()

    // Initial demo entries should render
    expect(wrapper.find('[data-testid="logs-demo-badge"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="log-row-2"]').exists()).toBe(true)

    const listEl = wrapper.find('[data-testid="logs-entry-list"]').element as HTMLElement
    Object.defineProperty(listEl, 'scrollHeight', { configurable: true, value: 960 })
    listEl.scrollTop = 0

    // Live simulation while unpaused autoscrolls to bottom
    await wrapper.find('[data-testid="logs-simulate-demo"]').trigger('click')
    await flushPromises()
    expect(listEl.scrollTop).toBe(960)

    // Pause stream, then simulate a 7-step demo voice turn
    Object.defineProperty(listEl, 'scrollHeight', { configurable: true, value: 1400 })
    listEl.scrollTop = 400
    await wrapper.find('[data-testid="logs-toggle-pause"]').trigger('click')
    expect(wrapper.find('[data-testid="logs-stream-status"]').text()).toBe('Paused')

    await wrapper.find('[data-testid="logs-simulate-demo"]').trigger('click')
    await flushPromises()
    expect(listEl.scrollTop).toBe(400)

    // While paused, new entries are queued and shown on both the toolbar and floating resume button
    expect(wrapper.find('[data-testid="logs-toggle-pause"]').text()).toContain('Resume (7 queued)')
    expect(wrapper.find('[data-testid="logs-floating-resume"]').exists()).toBe(true)

    // Resume via floating button flushes the queued entries into the console and autoscrolls to bottom
    await wrapper.find('[data-testid="logs-floating-resume"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-testid="logs-stream-status"]').text()).toBe('Live')
    expect(listEl.scrollTop).toBe(1400)

    // Expand row #2 and click one-click session correlation filter
    await wrapper.find('[data-testid="log-row-2"] > div').trigger('click')
    expect(wrapper.find('[data-testid="log-detail-2"]').exists()).toBe(true)
    await wrapper.find('[data-testid="filter-session-2"]').trigger('click')
    await flushPromises()

    expect(wrapper.find('[data-testid="logs-active-correlation"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="active-pill-session"]').text()).toContain(
      'session_id=sess-demo-101',
    )

    // Clear correlation filter
    await wrapper.find('[data-testid="logs-clear-correlation"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-testid="logs-active-correlation"]').exists()).toBe(false)

    // Switch to Admin Audit Trail tab and verify numeric Unix timestamp renders properly
    await wrapper.find('[data-testid="tab-audit-logs"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-testid="admin-audit-section"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="audit-row-104"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="audit-row-104"]').text()).toContain('req=req-audit-104')

    wrapper.unmount()
  })
})

import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { notificationsApi, resetDemoNotificationsForTest } from '@/api/notifications'
import { setLocale } from '@/composables/useI18n'
import NotificationSettingsCard from './NotificationSettingsCard.vue'

describe('NotificationSettingsCard.vue', () => {
  beforeEach(() => {
    setLocale('en')
    resetDemoNotificationsForTest()
    vi.stubGlobal(
      'fetch',
      vi.fn().mockRejectedValue(new Error('Server notifications not deployed yet')),
    )
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('loads demo channel settings and preserves saved Telegram token when input is left blank', async () => {
    const updateSpy = vi.spyOn(notificationsApi, 'updateSettings')
    const wrapper = mount(NotificationSettingsCard)
    await flushPromises()

    expect(wrapper.find('[data-testid="notification-settings-demo-banner"]').exists()).toBe(true)
    const chatIdInput = wrapper.get('[data-testid="telegram-chat-id"]')
    expect((chatIdInput.element as HTMLInputElement).value).toBe('-100208491032')

    // Change Telegram chat ID and toggle Zalo on with a new token
    await chatIdInput.setValue('-100555666')
    await wrapper.get('[data-testid="zalo-enabled-toggle"]').setValue(true)
    await wrapper.get('[data-testid="zalo-bot-token"]').setValue('zalo-token-9988')
    await wrapper.get('[data-testid="zalo-recipient-id"]').setValue('zalo-group-01')

    await wrapper.get('[data-testid="notification-settings-save"]').trigger('click')
    await flushPromises()

    expect(updateSpy).toHaveBeenCalledTimes(1)
    const [payload, rev] = updateSpy.mock.calls[0]!
    expect(rev).toBe(1)
    // Telegram token input was left blank -> undefined (keep existing saved token)
    expect(payload.telegram.bot_token).toBeUndefined()
    expect(payload.telegram.chat_id).toBe('-100555666')
    // Zalo token input had a new value -> string
    expect(payload.zalo.bot_token).toBe('zalo-token-9988')
    expect(payload.zalo.recipient_id).toBe('zalo-group-01')

    expect(wrapper.find('[data-testid="notification-settings-saved"]').text()).toContain(
      'revision 2',
    )
    wrapper.unmount()
  })

  it('supports clearing a saved bot token (bot_token: null) and undoing before save', async () => {
    const updateSpy = vi.spyOn(notificationsApi, 'updateSettings')
    const wrapper = mount(NotificationSettingsCard)
    await flushPromises()

    // Click Clear token on Telegram
    await wrapper.get('[data-testid="telegram-clear-token"]').trigger('click')
    expect(wrapper.find('[data-testid="telegram-clear-notice"]').exists()).toBe(true)

    // Undo clear token
    await wrapper.get('[data-testid="telegram-undo-clear-token"]').trigger('click')
    expect(wrapper.find('[data-testid="telegram-clear-notice"]').exists()).toBe(false)

    // Clear token again and save
    await wrapper.get('[data-testid="telegram-clear-token"]').trigger('click')
    await wrapper.get('[data-testid="notification-settings-save"]').trigger('click')
    await flushPromises()

    const [payload] = updateSpy.mock.calls[0]!
    expect(payload.telegram.bot_token).toBeNull()
    wrapper.unmount()
  })

  it('sends test messages for Telegram and Zalo channels and displays delivery feedback', async () => {
    const wrapper = mount(NotificationSettingsCard)
    await flushPromises()

    // Telegram has a saved demo token and chat ID out-of-the-box -> succeeds
    await wrapper.get('[data-testid="telegram-test-btn"]').trigger('click')
    await flushPromises()
    expect(wrapper.get('[data-testid="telegram-test-result"]').text()).toContain('demo-tg-')

    // Zalo has no saved token initially -> fails with descriptive message
    await wrapper.get('[data-testid="zalo-test-btn"]').trigger('click')
    await flushPromises()
    expect(wrapper.get('[data-testid="zalo-test-result"]').text()).toContain(
      'Missing saved Zalo Bot token',
    )
    wrapper.unmount()
  })
})

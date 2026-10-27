import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import {
  isNotificationsDemoMode,
  listenNotificationStream,
  notificationsApi,
  resetDemoNotificationsForTest,
  subscribeNotificationEvents,
} from './notifications'

function json(value: unknown, status = 200) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

describe('notificationsApi & stateful demo store fallback', () => {
  beforeEach(() => {
    resetDemoNotificationsForTest()
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('calls backend REST endpoints and sends If-Match when server is available', async () => {
    const pagePayload = {
      items: [
        {
          id: 'srv-1',
          topic: 'task_completed',
          severity: 'success',
          title: 'Server task done',
          body: 'Finished indexing',
          source_kind: 'job',
          source_id: 'task-1',
          metadata: {},
          is_read: false,
          read_at_ms: null,
          created_at_ms: 1792977120000,
          deliveries: [
            {
              channel: 'telegram',
              status: 'sent',
              attempts: 1,
              last_error_code: null,
              updated_at_ms: 1792977122000,
            },
          ],
        },
      ],
      total: 1,
      unread_count: 1,
      page: 1,
      page_size: 20,
    }
    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes('/api/admin/notifications/settings')) {
        return Promise.resolve(
          json({
            revision: 5,
            web_ui: { enabled: true, toast_enabled: true, retention_days: 30 },
            telegram: {
              enabled: true,
              chat_id: '-100123',
              thread_id: null,
              subscribed_topics: ['task_failed'],
              has_saved_token: true,
              has_env_fallback: false,
              credential_metadata: {
                id: 'cred-1',
                masked_key: '…1122',
                key_version: 1,
                status: 'active',
              },
            },
            zalo: {
              enabled: false,
              recipient_id: '',
              subscribed_topics: ['task_failed'],
              has_saved_token: false,
              has_env_fallback: false,
              credential_metadata: null,
            },
          }),
        )
      }
      if (url.includes('/api/admin/notifications/channels/telegram/test')) {
        return Promise.resolve(
          json({
            ok: true,
            channel: 'telegram',
            external_message_id: 'tg-msg-42',
            delivered_at_ms: 1792977125000,
          }),
        )
      }
      return Promise.resolve(json(pagePayload))
    })
    vi.stubGlobal('fetch', fetchMock)

    const page = await notificationsApi.list({ unread_only: true, page_size: 20 })
    expect(isNotificationsDemoMode()).toBe(false)
    expect(page.items[0]?.id).toBe('srv-1')
    expect(fetchMock.mock.calls[0]?.[0]).toContain('/api/admin/notifications')
    expect(fetchMock.mock.calls[0]?.[0]).toContain('unread_only=true')
    expect(fetchMock.mock.calls[0]?.[0]).toContain('page_size=20')

    await notificationsApi.updateSettings(
      {
        web_ui: { toast_enabled: false, retention_days: 14 },
        telegram: {
          enabled: true,
          chat_id: '-100123',
          subscribed_topics: ['task_failed'],
        },
        zalo: {
          enabled: false,
          recipient_id: '',
          subscribed_topics: ['task_failed'],
        },
      },
      4,
    )
    const putCall = fetchMock.mock.calls[1]
    expect(putCall?.[0]).toContain('/api/admin/notifications/settings')
    const init = putCall?.[1] as RequestInit
    expect(init.method).toBe('PUT')
    expect(new Headers(init.headers).get('if-match')).toBe('"4"')

    const testRes = await notificationsApi.testChannel('telegram', { message: 'Hello Telegram' })
    expect(testRes.ok).toBe(true)
    expect(testRes.external_message_id).toBe('tg-msg-42')
  })

  it('coalesces concurrent identical settings/list reads into one request each', async () => {
    const settingsPayload = {
      revision: 1,
      web_ui: { enabled: true, toast_enabled: true, retention_days: 30 },
      telegram: {
        enabled: false, chat_id: '', thread_id: null, subscribed_topics: [],
        has_saved_token: false, has_env_fallback: false, credential_metadata: null,
      },
      zalo: {
        enabled: false, recipient_id: '', subscribed_topics: [],
        has_saved_token: false, has_env_fallback: false, credential_metadata: null,
      },
    }
    const pagePayload = { items: [], total: 0, unread_count: 0, page: 1, page_size: 30 }
    const fetchMock = vi.fn().mockImplementation((url: string) =>
      Promise.resolve(json(url.includes('/settings') ? settingsPayload : pagePayload)),
    )
    vi.stubGlobal('fetch', fetchMock)

    const [a, b] = await Promise.all([notificationsApi.settings(), notificationsApi.settings()])
    await Promise.all([
      notificationsApi.list({ page_size: 30 }),
      notificationsApi.list({ page_size: 30 }),
      notificationsApi.list({ page_size: 30, unread_only: true }),
    ])

    const urls = fetchMock.mock.calls.map(call => String(call[0]))
    expect(urls.filter(url => url.includes('/settings'))).toHaveLength(1)
    expect(urls.filter(url => !url.includes('/settings'))).toHaveLength(2)
    expect(a).toEqual(b)
    expect(a).not.toBe(b)

    // Once settled, the next read hits the network again.
    await notificationsApi.settings()
    expect(fetchMock.mock.calls.filter(call => String(call[0]).includes('/settings'))).toHaveLength(2)
  })

  it('seamlessly falls back to stateful demo inbox when backend returns 404', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        json({ error: { code: 'not_found', message: 'Not implemented' } }, 404),
      ),
    )

    const initial = await notificationsApi.list()
    expect(isNotificationsDemoMode()).toBe(true)
    expect(initial.items.length).toBeGreaterThanOrEqual(5)
    expect(initial.unread_count).toBeGreaterThan(0)

    const unreadOnly = await notificationsApi.list({ unread_only: true })
    expect(unreadOnly.items.length).toBe(initial.unread_count)
    expect(unreadOnly.items.every(item => !item.is_read)).toBe(true)

    const firstUnread = unreadOnly.items[0]!
    await notificationsApi.markRead([firstUnread.id])

    const afterSingleRead = await notificationsApi.list()
    expect(afterSingleRead.unread_count).toBe(initial.unread_count - 1)
    expect(afterSingleRead.items.find(item => item.id === firstUnread.id)?.is_read).toBe(true)

    await notificationsApi.markAllRead()
    const afterAllRead = await notificationsApi.list()
    expect(afterAllRead.unread_count).toBe(0)

    // Simulate a new live demo notification and verify subscriber callback
    const createdEvents: string[] = []
    const unsubscribe = subscribeNotificationEvents({
      onCreated: item => createdEvents.push(item.id),
    })
    const simulated = await notificationsApi.simulateDemoNotification()
    unsubscribe()

    expect(createdEvents).toContain(simulated.id)
    const afterSimulated = await notificationsApi.list()
    expect(afterSimulated.unread_count).toBe(1)
    expect(afterSimulated.items[0]?.id).toBe(simulated.id)

    // Purge read removes read items while keeping the unread simulated item
    await notificationsApi.purgeRead()
    const afterPurge = await notificationsApi.list()
    expect(afterPurge.items.map(i => i.id)).toEqual([simulated.id])

    // Remove the remaining item
    await notificationsApi.remove(simulated.id)
    const afterRemove = await notificationsApi.list()
    expect(afterRemove.items).toHaveLength(0)
  })

  it('supports Patch<SecretValue> semantics and channel testing in demo mode', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockRejectedValue(new Error('Network offline')),
    )

    const initial = await notificationsApi.settings()
    expect(isNotificationsDemoMode()).toBe(true)
    expect(initial.telegram.has_saved_token).toBe(true)
    expect(initial.telegram.credential_metadata?.masked_key).toBeTruthy()
    expect(initial.zalo.has_saved_token).toBe(false)

    // 1. Omitting bot_token (undefined) preserves saved Telegram token, while setting a new Zalo token saves it
    const updated1 = await notificationsApi.updateSettings(
      {
        web_ui: {
          toast_enabled: initial.web_ui.toast_enabled,
          retention_days: initial.web_ui.retention_days,
        },
        telegram: {
          enabled: true,
          chat_id: '-100998877',
          thread_id: 12,
          subscribed_topics: ['task_failed', 'reminder_due'],
        },
        zalo: {
          enabled: true,
          bot_token: 'zalo-secret-token-7788',
          recipient_id: 'zalo-group-42',
          subscribed_topics: ['task_failed', 'system_change_request'],
        },
      },
      initial.revision,
    )

    expect(updated1.revision).toBe(initial.revision + 1)
    expect(updated1.telegram.has_saved_token).toBe(true)
    expect(updated1.telegram.chat_id).toBe('-100998877')
    expect(updated1.zalo.has_saved_token).toBe(true)
    expect(updated1.zalo.credential_metadata?.masked_key).toBe('…7788')

    // Test Zalo channel succeeds now that token & recipient ID are configured
    const zaloTest = await notificationsApi.testChannel('zalo')
    expect(zaloTest.ok).toBe(true)
    expect(zaloTest.external_message_id).toContain('demo-zalo-')

    // 2. Passing bot_token: null explicitly clears the saved Telegram token
    const updated2 = await notificationsApi.updateSettings(
      {
        web_ui: {
          toast_enabled: updated1.web_ui.toast_enabled,
          retention_days: updated1.web_ui.retention_days,
        },
        telegram: {
          enabled: false,
          bot_token: null,
          chat_id: '-100998877',
          subscribed_topics: ['task_failed'],
        },
        zalo: {
          enabled: true,
          recipient_id: 'zalo-group-42',
          subscribed_topics: ['task_failed'],
        },
      },
      updated1.revision,
    )

    expect(updated2.telegram.has_saved_token).toBe(false)
    expect(updated2.telegram.credential_metadata).toBeNull()

    // Testing Telegram without a saved token returns ok: false with clear reason
    const tgTestFail = await notificationsApi.testChannel('telegram')
    expect(tgTestFail.ok).toBe(false)
    expect(tgTestFail.error).toBeTruthy()
  })

  it('rethrows real server errors (409 Conflict, 401 Unauthorized) instead of faking demo success', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        json({ error: { code: 'revision_conflict', message: 'Stale revision' } }, 409),
      ),
    )

    await expect(
      notificationsApi.updateSettings(
        {
          web_ui: { toast_enabled: true, retention_days: 30 },
          telegram: { enabled: true, chat_id: '-1001', subscribed_topics: ['task_failed'] },
          zalo: { enabled: false, recipient_id: '', subscribed_topics: [] },
        },
        1,
      ),
    ).rejects.toMatchObject({ status: 409, code: 'revision_conflict' })
    expect(isNotificationsDemoMode()).toBe(false)
  })

  it('parses notification_created events from SSE stream when backend stream is active', async () => {
    const abort = new AbortController()
    const createdIds: string[] = []
    let changedCount = 0
    const encoder = new TextEncoder()
    const payload =
      'event: ready\r\ndata: {}\r\n\r\n' +
      'event: notification_created\r\ndata: {"id":"sse-1","topic":"task_failed","severity":"error","title":"Failed","body":"Err","source_kind":"job","source_id":"j1","metadata":{},"is_read":false,"read_at_ms":null,"created_at_ms":1,"deliveries":[]}\r\n\r\n'

    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(
          new ReadableStream<Uint8Array>({
            start(stream) {
              stream.enqueue(encoder.encode(payload))
              stream.close()
            },
          }),
          { headers: { 'Content-Type': 'text/event-stream' } },
        ),
      ),
    )

    await listenNotificationStream(
      {
        onCreated: item => {
          createdIds.push(item.id)
          abort.abort()
        },
        onChanged: () => {
          changedCount += 1
        },
      },
      abort.signal,
    )

    expect(createdIds).toEqual(['sse-1'])
    expect(changedCount).toBeGreaterThanOrEqual(2)
  })
})

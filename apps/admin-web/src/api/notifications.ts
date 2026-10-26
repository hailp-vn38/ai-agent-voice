import { apiUrl, jsonRequest, requestJson, withQuery } from '@/api/client'
import { isApiError } from '@/api/errors'
import type {
  ChannelCredentialMetadata,
  ChannelDeliveryStatus,
  ExternalChannelKind,
  MarkNotificationsReadResult,
  NotificationInboxPage,
  NotificationItem,
  NotificationListQuery,
  NotificationSettings,
  NotificationSettingsUpdate,
  NotificationTopic,
  PurgeNotificationsResult,
  TestChannelInput,
  TestChannelOutput,
} from '@/api/types/notifications'
import { readAdminToken } from '@/stores/auth'

export interface NotificationStreamHandlers {
  onCreated?: (item: NotificationItem) => void
  onChanged?: () => void
}

const BASE_TIME_MS = 1792977120000

export function resolveNotificationHref(item: NotificationItem): string | null {
  if (typeof item.metadata?.resource_href === 'string' && item.metadata.resource_href) {
    return item.metadata.resource_href
  }
  if (item.source_kind === 'job') {
    return item.source_id ? `/jobs/${encodeURIComponent(item.source_id)}` : '/jobs'
  }
  if (item.source_kind === 'schedule') {
    return '/jobs'
  }
  if (item.source_kind === 'wiki') {
    return '/knowledge'
  }
  if (item.source_kind === 'system') {
    return '/system'
  }
  return null
}

function buildInitialDemoSettings(): NotificationSettings {
  return {
    revision: 1,
    web_ui: {
      enabled: true,
      toast_enabled: true,
      retention_days: 30,
    },
    telegram: {
      enabled: true,
      chat_id: '-100208491032',
      thread_id: 4,
      subscribed_topics: [
        'reminder_due',
        'task_failed',
        'wiki_review_required',
        'system_change_request',
      ],
      has_saved_token: true,
      has_env_fallback: false,
      credential_metadata: {
        id: 'cred-tg-demo',
        masked_key: '…9482',
        key_version: 1,
        status: 'active',
      },
    },
    zalo: {
      enabled: false,
      recipient_id: '',
      subscribed_topics: ['task_failed', 'system_change_request'],
      has_saved_token: false,
      has_env_fallback: false,
      credential_metadata: null,
    },
    demo_mode: true,
  }
}

function buildInitialDemoItems(): NotificationItem[] {
  return [
    {
      id: 'notif-demo-1',
      topic: 'task_failed',
      severity: 'error',
      title: 'Background Task failed: Sync Q4 customer pricing',
      body: 'Worker timed out after 30s while fetching external MCP pricing sheet.',
      source_kind: 'job',
      source_id: 'job-demo-pricing-01',
      metadata: { attempt: 2, worker: 'wiki_researcher' },
      is_read: false,
      read_at_ms: null,
      created_at_ms: BASE_TIME_MS,
      deliveries: [
        {
          channel: 'telegram',
          status: 'sent',
          attempts: 1,
          last_error_code: null,
          updated_at_ms: BASE_TIME_MS + 2000,
        },
      ],
    },
    {
      id: 'notif-demo-2',
      topic: 'reminder_due',
      severity: 'warning',
      title: 'Reminder due on Reception Desk ESP32',
      body: 'Confirm firmware OTA rollout window with operations team at 09:00.',
      source_kind: 'schedule',
      source_id: 'rem-demo-104',
      metadata: { device_id: 'esp32-reception-01', resource_href: '/devices' },
      is_read: false,
      read_at_ms: null,
      created_at_ms: BASE_TIME_MS - 600_000,
      deliveries: [
        {
          channel: 'telegram',
          status: 'sent',
          attempts: 1,
          last_error_code: null,
          updated_at_ms: BASE_TIME_MS - 599_000,
        },
      ],
    },
    {
      id: 'notif-demo-3',
      topic: 'wiki_review_required',
      severity: 'warning',
      title: 'Wiki draft ready for review: Voice Pipeline Troubleshooting',
      body: 'Agent wiki_maintainer synthesized 3 new field reports into a candidate draft.',
      source_kind: 'wiki',
      source_id: 'wiki-draft-voice-pipeline',
      metadata: {
        draft_path: 'ops/voice-pipeline-troubleshooting.md',
        resource_href: '/knowledge',
      },
      is_read: false,
      read_at_ms: null,
      created_at_ms: BASE_TIME_MS - 3_600_000,
      deliveries: [
        {
          channel: 'telegram',
          status: 'sent',
          attempts: 1,
          last_error_code: null,
          updated_at_ms: BASE_TIME_MS - 3_598_000,
        },
      ],
    },
    {
      id: 'notif-demo-4',
      topic: 'system_change_request',
      severity: 'warning',
      title: 'Studio Assistant proposed an MCP configuration change',
      body: 'Pending Admin approval to enable tool discovery on server crm_connector.',
      source_kind: 'system',
      source_id: 'req-demo-88',
      metadata: { target: 'crm_connector', resource_href: '/system' },
      is_read: true,
      read_at_ms: BASE_TIME_MS - 7_000_000,
      created_at_ms: BASE_TIME_MS - 7_200_000,
      deliveries: [
        {
          channel: 'telegram',
          status: 'sent',
          attempts: 1,
          last_error_code: null,
          updated_at_ms: BASE_TIME_MS - 7_198_000,
        },
      ],
    },
    {
      id: 'notif-demo-5',
      topic: 'task_completed',
      severity: 'success',
      title: 'Background Task completed: Daily Knowledge index check',
      body: 'Verified 48 markdown notes and 0 broken wikilinks across vault.',
      source_kind: 'job',
      source_id: 'job-demo-index-09',
      metadata: { notes_checked: 48 },
      is_read: true,
      read_at_ms: BASE_TIME_MS - 14_000_000,
      created_at_ms: BASE_TIME_MS - 14_400_000,
      deliveries: [],
    },
    {
      id: 'notif-demo-6',
      topic: 'schedule_triggered',
      severity: 'info',
      title: 'Recurring schedule triggered: Nightly Memory Compaction',
      body: 'Dispatched background flow run for schedule nightly_memory_compaction.',
      source_kind: 'schedule',
      source_id: 'sched-nightly-compact',
      metadata: { cron: '0 2 * * *' },
      is_read: true,
      read_at_ms: BASE_TIME_MS - 18_000_000,
      created_at_ms: BASE_TIME_MS - 18_200_000,
      deliveries: [],
    },
  ]
}

const DEMO_SIMULATION_TEMPLATES: Array<{
  topic: NotificationTopic
  severity: NotificationItem['severity']
  title: string
  body: string
  source_kind: NotificationItem['source_kind']
  source_id: string
  resource_href?: string
}> = [
  {
    topic: 'reminder_due',
    severity: 'warning',
    title: 'Reminder due: Inspect Meeting Room B microphone gain',
    body: 'Scheduled acoustic check on device esp32-meeting-b is due now.',
    source_kind: 'schedule',
    source_id: 'rem-sim-201',
    resource_href: '/devices',
  },
  {
    topic: 'task_failed',
    severity: 'error',
    title: 'Background Task failed: Export weekly transcript digest',
    body: 'Webhook endpoint returned HTTP 502 Bad Gateway after 3 retries.',
    source_kind: 'job',
    source_id: 'job-sim-502',
  },
  {
    topic: 'wiki_review_required',
    severity: 'warning',
    title: 'Wiki draft awaiting review: Zalo & Telegram Alert Runbook',
    body: 'New operational runbook draft requires Admin verification before publish.',
    source_kind: 'wiki',
    source_id: 'wiki-sim-runbook',
    resource_href: '/knowledge',
  },
  {
    topic: 'task_completed',
    severity: 'success',
    title: 'Background Task completed: Rebuild speaker voiceprint cache',
    body: 'Updated 12 active speaker embeddings in 1.4s.',
    source_kind: 'job',
    source_id: 'job-sim-speakers',
  },
]

let demoMode = false
let demoSettings: NotificationSettings = buildInitialDemoSettings()
let demoItems: NotificationItem[] = buildInitialDemoItems()
let demoCounter = 100
const subscribers = new Set<NotificationStreamHandlers>()

export function isNotificationsDemoMode(): boolean {
  return demoMode
}

export function resetDemoNotificationsForTest(): void {
  demoMode = false
  demoSettings = buildInitialDemoSettings()
  demoItems = buildInitialDemoItems()
  demoCounter = 100
  subscribers.clear()
}

function shouldFallbackToDemo(err: unknown): boolean {
  if (isApiError(err)) {
    return err.status === 404 || err.status === 501
  }
  return true
}

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

function notifyCreated(item: NotificationItem): void {
  const snapshot = clone(item)
  for (const sub of subscribers) {
    sub.onCreated?.(snapshot)
    sub.onChanged?.()
  }
}

function notifyChanged(): void {
  for (const sub of subscribers) {
    sub.onChanged?.()
  }
}

function isValidInboxPage(value: unknown): value is NotificationInboxPage {
  if (!value || typeof value !== 'object') return false
  const candidate = value as Record<string, unknown>
  return Array.isArray(candidate.items) && typeof candidate.unread_count === 'number'
}

function isValidSettings(value: unknown): value is NotificationSettings {
  if (!value || typeof value !== 'object') return false
  const candidate = value as Record<string, unknown>
  return (
    typeof candidate.revision === 'number' &&
    typeof candidate.web_ui === 'object' &&
    candidate.web_ui !== null &&
    typeof candidate.telegram === 'object' &&
    candidate.telegram !== null &&
    typeof candidate.zalo === 'object' &&
    candidate.zalo !== null
  )
}

function computeDemoInboxPage(params?: NotificationListQuery): NotificationInboxPage {
  const unreadCount = demoItems.filter(item => !item.is_read).length
  let filtered = demoItems.slice()
  if (params?.unread_only) {
    filtered = filtered.filter(item => !item.is_read)
  }
  if (params?.topic) {
    filtered = filtered.filter(item => item.topic === params.topic)
  }
  const page = params?.page ?? 1
  const pageSize = params?.page_size ?? 30
  const start = (page - 1) * pageSize
  const paged = filtered.slice(start, start + pageSize)
  return {
    items: clone(paged),
    total: filtered.length,
    unread_count: unreadCount,
    page,
    page_size: pageSize,
    demo_mode: true,
  }
}

function maskTokenHint(rawToken: string): string {
  const trimmed = rawToken.trim()
  const tail = trimmed.slice(-4) || '****'
  return `…${tail}`
}

function applySecretPatch(
  credentialId: string,
  currentSaved: boolean,
  currentMeta: ChannelCredentialMetadata | null,
  patch: string | null | undefined,
): { has_saved_token: boolean; credential_metadata: ChannelCredentialMetadata | null } {
  if (patch === null) {
    return { has_saved_token: false, credential_metadata: null }
  }
  if (typeof patch === 'string' && patch.trim()) {
    return {
      has_saved_token: true,
      credential_metadata: {
        id: credentialId,
        masked_key: maskTokenHint(patch),
        key_version: (currentMeta?.key_version ?? 0) + 1,
        status: 'active',
      },
    }
  }
  return { has_saved_token: currentSaved, credential_metadata: currentMeta }
}

function buildDeliveriesForTopic(topic: NotificationTopic, nowMs: number): ChannelDeliveryStatus[] {
  const deliveries: ChannelDeliveryStatus[] = []
  if (demoSettings.telegram.subscribed_topics.includes(topic) && demoSettings.telegram.enabled) {
    const canSend =
      demoSettings.telegram.has_saved_token && Boolean(demoSettings.telegram.chat_id.trim())
    deliveries.push({
      channel: 'telegram',
      status: canSend ? 'sent' : 'failed',
      attempts: 1,
      last_error_code: canSend ? null : 'missing_credentials',
      updated_at_ms: nowMs,
    })
  }
  if (demoSettings.zalo.subscribed_topics.includes(topic) && demoSettings.zalo.enabled) {
    const canSend =
      demoSettings.zalo.has_saved_token && Boolean(demoSettings.zalo.recipient_id.trim())
    deliveries.push({
      channel: 'zalo',
      status: canSend ? 'sent' : 'failed',
      attempts: 1,
      last_error_code: canSend ? null : 'missing_credentials',
      updated_at_ms: nowMs,
    })
  }
  return deliveries
}

export const notificationsApi = {
  async list(params: NotificationListQuery = {}): Promise<NotificationInboxPage> {
    try {
      const response = await requestJson<NotificationInboxPage>(
        withQuery('/api/admin/notifications', {
          page: params.page ?? 1,
          page_size: params.page_size ?? 30,
          unread_only: params.unread_only ? 'true' : undefined,
          topic: params.topic,
        }),
      )
      if (isValidInboxPage(response)) {
        demoMode = Boolean(response.demo_mode)
        return response
      }
    } catch (err) {
      if (!shouldFallbackToDemo(err)) throw err
    }
    demoMode = true
    return computeDemoInboxPage(params)
  },

  async markRead(ids: string[]): Promise<MarkNotificationsReadResult> {
    if (!demoMode) {
      try {
        const res = await requestJson<MarkNotificationsReadResult>(
          '/api/admin/notifications/read',
          jsonRequest('POST', { ids, all: false }),
        )
        notifyChanged()
        return res
      } catch (err) {
        if (!shouldFallbackToDemo(err)) throw err
        demoMode = true
      }
    }
    const nowMs = Date.now()
    const idSet = new Set(ids)
    let updated = 0
    demoItems = demoItems.map(item => {
      if (idSet.has(item.id) && !item.is_read) {
        updated += 1
        return { ...item, is_read: true, read_at_ms: nowMs }
      }
      return item
    })
    notifyChanged()
    return {
      updated,
      unread_count: demoItems.filter(item => !item.is_read).length,
    }
  },

  async markAllRead(): Promise<MarkNotificationsReadResult> {
    if (!demoMode) {
      try {
        const res = await requestJson<MarkNotificationsReadResult>(
          '/api/admin/notifications/read',
          jsonRequest('POST', { ids: [], all: true }),
        )
        notifyChanged()
        return res
      } catch (err) {
        if (!shouldFallbackToDemo(err)) throw err
        demoMode = true
      }
    }
    const nowMs = Date.now()
    let updated = 0
    demoItems = demoItems.map(item => {
      if (!item.is_read) {
        updated += 1
        return { ...item, is_read: true, read_at_ms: nowMs }
      }
      return item
    })
    notifyChanged()
    return {
      updated,
      unread_count: 0,
    }
  },

  async remove(id: string): Promise<void> {
    if (!demoMode) {
      try {
        await requestJson<void>(`/api/admin/notifications/${encodeURIComponent(id)}`, {
          method: 'DELETE',
        })
        notifyChanged()
        return
      } catch (err) {
        if (!shouldFallbackToDemo(err)) throw err
        demoMode = true
      }
    }
    demoItems = demoItems.filter(item => item.id !== id)
    notifyChanged()
  },

  async purgeRead(): Promise<PurgeNotificationsResult> {
    if (!demoMode) {
      try {
        const res = await requestJson<PurgeNotificationsResult>(
          '/api/admin/notifications/purge',
          jsonRequest('POST', { read_only: true }),
        )
        notifyChanged()
        return res
      } catch (err) {
        if (!shouldFallbackToDemo(err)) throw err
        demoMode = true
      }
    }
    const before = demoItems.length
    demoItems = demoItems.filter(item => !item.is_read)
    notifyChanged()
    return {
      deleted: before - demoItems.length,
      unread_count: demoItems.length,
    }
  },

  async settings(): Promise<NotificationSettings> {
    try {
      const response = await requestJson<NotificationSettings>('/api/admin/notifications/settings')
      if (isValidSettings(response)) {
        demoMode = Boolean(response.demo_mode)
        return response
      }
    } catch (err) {
      if (!shouldFallbackToDemo(err)) throw err
    }
    demoMode = true
    return clone(demoSettings)
  },

  async updateSettings(
    input: NotificationSettingsUpdate,
    revision: number,
  ): Promise<NotificationSettings> {
    if (!demoMode) {
      try {
        const response = await requestJson<NotificationSettings>(
          '/api/admin/notifications/settings',
          jsonRequest('PUT', input),
          { revision },
        )
        if (isValidSettings(response)) {
          notifyChanged()
          return response
        }
      } catch (err) {
        if (!shouldFallbackToDemo(err)) throw err
        demoMode = true
      }
    }

    demoMode = true

    const tgSecret = applySecretPatch(
      'cred-tg-demo',
      demoSettings.telegram.has_saved_token,
      demoSettings.telegram.credential_metadata,
      input.telegram.bot_token,
    )
    const zaloSecret = applySecretPatch(
      'cred-zalo-demo',
      demoSettings.zalo.has_saved_token,
      demoSettings.zalo.credential_metadata,
      input.zalo.bot_token,
    )

    demoSettings = {
      revision: revision + 1,
      web_ui: {
        enabled: true,
        toast_enabled: input.web_ui.toast_enabled,
        retention_days: Math.max(1, Math.min(365, Math.round(input.web_ui.retention_days || 30))),
      },
      telegram: {
        enabled: input.telegram.enabled,
        chat_id: input.telegram.chat_id.trim(),
        thread_id: input.telegram.thread_id ?? null,
        subscribed_topics: [...input.telegram.subscribed_topics],
        has_saved_token: tgSecret.has_saved_token,
        has_env_fallback: demoSettings.telegram.has_env_fallback,
        credential_metadata: tgSecret.credential_metadata,
      },
      zalo: {
        enabled: input.zalo.enabled,
        recipient_id: input.zalo.recipient_id.trim(),
        subscribed_topics: [...input.zalo.subscribed_topics],
        has_saved_token: zaloSecret.has_saved_token,
        has_env_fallback: demoSettings.zalo.has_env_fallback,
        credential_metadata: zaloSecret.credential_metadata,
      },
      demo_mode: true,
    }

    notifyChanged()
    return clone(demoSettings)
  },

  async testChannel(
    channel: ExternalChannelKind,
    input: TestChannelInput = {},
  ): Promise<TestChannelOutput> {
    if (!demoMode) {
      try {
        const response = await requestJson<TestChannelOutput>(
          `/api/admin/notifications/channels/${channel}/test`,
          jsonRequest('POST', input),
        )
        if (response && typeof response.ok === 'boolean') {
          return response
        }
      } catch (err) {
        if (!shouldFallbackToDemo(err)) throw err
        demoMode = true
      }
    }

    demoMode = true
    const nowMs = Date.now()
    const isTelegram = channel === 'telegram'
    const targetId = isTelegram
      ? (input.chat_id ?? demoSettings.telegram.chat_id).trim()
      : (input.recipient_id ?? demoSettings.zalo.recipient_id).trim()
    const channelConfig = isTelegram ? demoSettings.telegram : demoSettings.zalo
    const hasToken =
      Boolean(input.bot_token?.trim()) ||
      channelConfig.has_saved_token ||
      channelConfig.has_env_fallback

    if (!hasToken) {
      return {
        ok: false,
        channel,
        external_message_id: null,
        delivered_at_ms: nowMs,
        error: isTelegram
          ? 'Missing saved Telegram Bot token. Enter a bot token and save first.'
          : 'Missing saved Zalo Bot token. Enter a bot token and save first.',
        demo_mode: true,
      }
    }
    if (!targetId) {
      return {
        ok: false,
        channel,
        external_message_id: null,
        delivered_at_ms: nowMs,
        error: isTelegram
          ? 'Missing Telegram default chat ID.'
          : 'Missing Zalo recipient user/group ID.',
        demo_mode: true,
      }
    }
    demoCounter += 1
    return {
      ok: true,
      channel,
      external_message_id: `demo-${isTelegram ? 'tg' : 'zalo'}-${demoCounter}`,
      delivered_at_ms: nowMs,
      error: null,
      demo_mode: true,
    }
  },

  async simulateDemoNotification(): Promise<NotificationItem> {
    demoMode = true
    demoCounter += 1
    const template =
      DEMO_SIMULATION_TEMPLATES[demoCounter % DEMO_SIMULATION_TEMPLATES.length]!
    const nowMs = Date.now()
    const created: NotificationItem = {
      id: `notif-demo-${demoCounter}`,
      topic: template.topic,
      severity: template.severity,
      title: template.title,
      body: template.body,
      source_kind: template.source_kind,
      source_id: template.source_id,
      metadata: {
        demo: true,
        sequence: demoCounter,
        ...(template.resource_href ? { resource_href: template.resource_href } : {}),
      },
      is_read: false,
      read_at_ms: null,
      created_at_ms: nowMs,
      deliveries: buildDeliveriesForTopic(template.topic, nowMs),
    }
    demoItems = [created, ...demoItems]
    notifyCreated(created)
    return clone(created)
  },
}

export function subscribeNotificationEvents(handlers: NotificationStreamHandlers): () => void {
  subscribers.add(handlers)
  return () => {
    subscribers.delete(handlers)
  }
}

export async function listenNotificationStream(
  handlers: NotificationStreamHandlers,
  signal: AbortSignal,
): Promise<void> {
  const unsubscribe = subscribeNotificationEvents(handlers)
  try {
    const token = readAdminToken()
    const response = await fetch(apiUrl('/api/admin/notifications/stream'), {
      headers: {
        Accept: 'text/event-stream',
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
      },
      cache: 'no-store',
      signal,
    })
    if (
      !response.ok ||
      !response.body ||
      !response.headers.get('content-type')?.startsWith('text/event-stream')
    ) {
      return
    }
    const reader = response.body.getReader()
    const decoder = new TextDecoder()
    let buffer = ''
    let eventName = ''
    let dataLines: string[] = []
    try {
      while (!signal.aborted) {
        const { done, value } = await reader.read()
        if (done) break
        buffer += decoder.decode(value, { stream: true }).replace(/\r\n?/g, '\n')
        let newline: number
        while ((newline = buffer.indexOf('\n')) !== -1) {
          const line = buffer.slice(0, newline)
          buffer = buffer.slice(newline + 1)
          if (line === '') {
            if (eventName === 'notification_created') {
              try {
                const parsed = JSON.parse(dataLines.join('\n')) as NotificationItem
                if (parsed && typeof parsed.id === 'string') {
                  handlers.onCreated?.(parsed)
                }
              } catch {
                // Ignore malformed SSE payload; trigger resync below.
              }
              handlers.onChanged?.()
            } else if (
              eventName === 'unread_count_changed' ||
              eventName === 'resync_required' ||
              eventName === 'ready'
            ) {
              handlers.onChanged?.()
            }
            eventName = ''
            dataLines = []
          } else if (line.startsWith('event:')) {
            eventName = line.slice(6).trimStart()
          } else if (line.startsWith('data:')) {
            dataLines.push(line.slice(5).trimStart())
          }
        }
      }
    } finally {
      reader.releaseLock()
    }
  } catch {
    // When backend SSE endpoint is not deployed yet, in-memory demo pub/sub remains active.
  } finally {
    signal.addEventListener('abort', () => unsubscribe(), { once: true })
  }
}

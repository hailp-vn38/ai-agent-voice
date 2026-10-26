export type NotificationTopic =
  | 'reminder_due'
  | 'task_completed'
  | 'task_failed'
  | 'schedule_triggered'
  | 'wiki_review_required'
  | 'system_change_request'

export type NotificationSeverity = 'info' | 'success' | 'warning' | 'error'
export type NotificationSourceKind = 'job' | 'schedule' | 'wiki' | 'system'
export type ExternalChannelKind = 'telegram' | 'zalo'
export type OutboxDeliveryStatus = 'pending' | 'sending' | 'sent' | 'failed'

export const ALL_NOTIFICATION_TOPICS: readonly NotificationTopic[] = [
  'reminder_due',
  'task_completed',
  'task_failed',
  'schedule_triggered',
  'wiki_review_required',
  'system_change_request',
] as const

export interface ChannelDeliveryStatus {
  channel: ExternalChannelKind
  status: OutboxDeliveryStatus
  attempts: number
  last_error_code: string | null
  updated_at_ms: number
}

export interface NotificationItem {
  id: string
  topic: NotificationTopic
  severity: NotificationSeverity
  title: string
  body: string
  source_kind: NotificationSourceKind | null
  source_id: string | null
  metadata: Record<string, unknown>
  is_read: boolean
  read_at_ms: number | null
  created_at_ms: number
  deliveries: ChannelDeliveryStatus[]
}

export interface NotificationInboxPage {
  items: NotificationItem[]
  total: number
  unread_count: number
  page: number
  page_size: number
  demo_mode?: boolean
}

export interface NotificationListQuery {
  page?: number
  page_size?: number
  unread_only?: boolean
  topic?: NotificationTopic
}

export interface MarkNotificationsReadResult {
  updated: number
  unread_count: number
}

export interface PurgeNotificationsResult {
  deleted: number
  unread_count: number
}

export interface ChannelCredentialMetadata {
  id: string
  masked_key: string
  key_version: number
  status: string
}

export interface TelegramChannelSettings {
  enabled: boolean
  chat_id: string
  thread_id: number | null
  subscribed_topics: NotificationTopic[]
  has_saved_token: boolean
  has_env_fallback: boolean
  credential_metadata: ChannelCredentialMetadata | null
}

export interface ZaloChannelSettings {
  enabled: boolean
  recipient_id: string
  subscribed_topics: NotificationTopic[]
  has_saved_token: boolean
  has_env_fallback: boolean
  credential_metadata: ChannelCredentialMetadata | null
}

export interface NotificationSettings {
  revision: number
  web_ui: {
    enabled: true
    toast_enabled: boolean
    retention_days: number
  }
  telegram: TelegramChannelSettings
  zalo: ZaloChannelSettings
  demo_mode?: boolean
}

export interface NotificationSettingsUpdate {
  web_ui: {
    toast_enabled: boolean
    retention_days: number
  }
  telegram: {
    enabled: boolean
    chat_id: string
    thread_id?: number | null
    subscribed_topics: NotificationTopic[]
    /**
     * Patch<SecretValue> semantics:
     * - undefined (absent): retain existing saved token
     * - null: clear saved token
     * - string: seal & save new token
     */
    bot_token?: string | null
  }
  zalo: {
    enabled: boolean
    recipient_id: string
    subscribed_topics: NotificationTopic[]
    bot_token?: string | null
  }
}

export interface TestChannelInput {
  chat_id?: string
  thread_id?: number | null
  recipient_id?: string
  bot_token?: string | null
  message?: string
}

export interface TestChannelOutput {
  ok: boolean
  channel: ExternalChannelKind
  external_message_id: string | null
  delivered_at_ms: number
  error?: string | null
  demo_mode?: boolean
}

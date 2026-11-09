import { knowledgeMessages } from './i18n'

export type SyncDisabledReason = 'no_git' | 'not_configured' | 'read_only'
export type SyncErrorKind = 'git_missing' | 'read_only' | 'auth' | 'network' | 'timeout' | 'file_too_large' | 'unsafe_remote' | 'git' | 'vault'
export type SyncStatus =
  | { type: 'disabled'; reason: SyncDisabledReason }
  | { type: 'idle'; last_sync: number | null; ahead: number; behind: number }
  | { type: 'syncing' }
  | { type: 'deferred'; reason: 'ingest_running'; retry_at: number }
  | { type: 'conflicted'; copies: Array<{ scope: 'raw' | 'wiki'; relative: string }> }
  | { type: 'error'; kind: SyncErrorKind; message: string }

export type SyncReport = {
  committed: number
  pulled: Array<{ scope: 'raw' | 'wiki'; relative: string }>
  conflicts: Array<{ scope: 'raw' | 'wiki'; relative: string }>
  pushed: boolean
}

export type SyncSettingsInput = {
  remote_url: string
  branch: string
  auto: boolean
  interval_min: number
}

export type SyncConfiguration = SyncSettingsInput & { public_github_remote?: boolean }

export interface SyncBackend {
  readonly configurable: boolean
  status(): Promise<SyncStatus>
  syncNow(): Promise<SyncReport>
  configuration?(): Promise<SyncConfiguration>
  configure?(input: SyncSettingsInput): Promise<SyncStatus>
  disable?(): Promise<void>
  onStatus(listener: (status: SyncStatus) => void): () => void
}

type Locale = 'en' | 'vi'
type SyncMessageKey = keyof typeof knowledgeMessages.en

function translate(key: SyncMessageKey, locale: Locale, values: Record<string, string | number> = {}) {
  let result: string = knowledgeMessages[locale][key]
  for (const [name, value] of Object.entries(values)) result = result.replaceAll(`{${name}}`, String(value))
  return result
}

export function syncStatusLabel(status: SyncStatus, locale: Locale = 'en'): string {
  switch (status.type) {
    case 'disabled':
      return translate(status.reason === 'read_only' ? 'knowledgeSync.readOnly'
        : status.reason === 'no_git' ? 'knowledgeSync.noGit' : 'knowledgeSync.notConfigured', locale)
    case 'syncing': return translate('knowledgeSync.syncing', locale)
    case 'deferred': return translate('knowledgeSync.ingestRunning', locale)
    case 'conflicted': return translate('knowledgeSync.conflicts', locale, { count: status.copies.length })
    case 'error': return syncErrorMessage({ type: status.kind }, locale)
    case 'idle':
      if (status.ahead || status.behind) {
        return translate('knowledgeSync.aheadBehind', locale, { ahead: status.ahead, behind: status.behind })
      }
      if (status.last_sync == null) return translate('knowledgeSync.notSyncedYet', locale)
      return translate('knowledgeSync.syncedAt', locale, { time: new Date(status.last_sync * 1000).toLocaleTimeString(locale) })
  }
}

export function syncErrorMessage(error: unknown, locale: Locale = 'en'): string {
  const value = error && typeof error === 'object' ? error as { type?: unknown; kind?: unknown; code?: unknown } : {}
  const kind = String(value.kind ?? value.type ?? value.code ?? 'git').replace(/^sync_/, '')
  const keys: Record<string, SyncMessageKey> = {
    auth: 'knowledgeSync.error.auth', git_missing: 'knowledgeSync.error.git_missing',
    no_git: 'knowledgeSync.error.git_missing', unsafe_remote: 'knowledgeSync.error.unsafe_remote',
    network: 'knowledgeSync.error.network', read_only: 'knowledgeSync.error.read_only',
    timeout: 'knowledgeSync.error.timeout', file_too_large: 'knowledgeSync.error.file_too_large',
  }
  return translate(keys[kind] ?? 'knowledgeSync.error.generic', locale)
}

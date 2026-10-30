import { request, requestJson, requestText, withQuery } from './client'
import type {
  BackupFormat,
  BackupImportOptions,
  BackupImportReport,
  BackupInspectionReport,
  BackupScope,
  SystemResources,
  SystemStatus,
} from './types/system'

function postBackupPayload<T>(
  path: string,
  payload: Blob | ArrayBuffer | Uint8Array,
  options: BackupImportOptions,
  signal?: AbortSignal,
) {
  return requestJson<T>(
    withQuery(path, {
      mode: options.mode,
      scope: options.scope,
      clear_unreadable_credentials: options.clear_unreadable_credentials,
    }),
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/octet-stream' },
      body: payload as BodyInit,
    },
    { signal },
  )
}

export const systemApi = {
  resources(signal?: AbortSignal) {
    return requestJson<SystemResources>('/api/admin/system/resources', {}, { signal })
  },
  health(signal?: AbortSignal) {
    return requestText('/health', { headers: { Accept: 'text/plain' } }, { signal })
  },
  ready(signal?: AbortSignal) {
    return requestText('/ready', { headers: { Accept: 'text/plain' } }, { signal })
  },
  status(signal?: AbortSignal) {
    return requestJson<SystemStatus>('/api/admin/system', {}, { signal })
  },
  async exportBackup(format: BackupFormat = 'json', scope: BackupScope = 'full', signal?: AbortSignal) {
    const response = await request(
      withQuery('/api/admin/system/backup/export', { format, scope }),
      { method: 'GET' },
      { signal },
    )
    const disposition = response.headers.get('content-disposition') ?? ''
    const match = disposition.match(/filename="([^"]+)"/)
    const fallbackName = `voice-agent-backup.${format === 'sqlite' ? 'db' : 'json'}`
    return {
      blob: await response.blob(),
      filename: match?.[1] ?? fallbackName,
    }
  },
  inspectBackup(payload: Blob | ArrayBuffer | Uint8Array, options: BackupImportOptions, signal?: AbortSignal) {
    return postBackupPayload<BackupInspectionReport>('/api/admin/system/backup/inspect', payload, options, signal)
  },
  importBackup(payload: Blob | ArrayBuffer | Uint8Array, options: BackupImportOptions, signal?: AbortSignal) {
    return postBackupPayload<BackupImportReport>('/api/admin/system/backup/import', payload, options, signal)
  },
}

import { apiUrl } from './client'
import { consumeSse, createSseParserState } from './sse'
import { readAdminWebToken } from '@/stores/auth'
import type { SyncStatus } from '@vela/knowledge-ui'
import { handleSessionResponse } from './session'

export interface KnowledgeChangeCallbacks {
  /** The vault changed. `paths` is empty when the server could not list every path. */
  onChanged: (paths: string[]) => void
  /** Changes may have been missed while disconnected; reload everything. */
  onResync: () => void
  onSyncStatus?: (status: SyncStatus) => void
}

export type ListenKnowledgeChanges = (callbacks: KnowledgeChangeCallbacks, signal: AbortSignal) => Promise<void>

function pause(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise(resolve => {
    if (signal.aborted) return resolve()
    const finish = () => { clearTimeout(timer); signal.removeEventListener('abort', finish); resolve() }
    const timer = setTimeout(finish, ms)
    signal.addEventListener('abort', finish, { once: true })
  })
}

function changedPaths(data: string): string[] | null {
  try {
    const payload: unknown = JSON.parse(data)
    if (!payload || typeof payload !== 'object' || !('paths' in payload) || !Array.isArray(payload.paths)) return null
    if ('truncated' in payload && payload.truncated === true) return []
    return payload.paths.filter((path): path is string => typeof path === 'string')
  } catch {
    return null
  }
}

/** Subscribe to vault change hints pushed by the server; REST remains the authoritative read. */
export const listenKnowledgeChanges: ListenKnowledgeChanges = async (callbacks, signal) => {
  let connectedBefore = false
  let delay = 1000
  while (!signal.aborted) {
    try {
      const token = readAdminWebToken()
      const response = await fetch(apiUrl('/api/admin/knowledge/events'), {
        headers: {
          Accept: 'text/event-stream',
          ...(token ? { Authorization: `Bearer ${token}` } : {}),
        },
        cache: 'no-store',
        signal,
      })
      await handleSessionResponse(response)
      if (response.status === 401 || response.status === 403) return
      if (!response.ok || !response.body ||
        !response.headers.get('content-type')?.startsWith('text/event-stream')) {
        throw new Error('knowledge_sse_unavailable')
      }
      const reader = response.body.getReader()
      const decoder = new TextDecoder()
      const state = createSseParserState()
      try {
        while (!signal.aborted) {
          const { done, value } = await reader.read()
          if (done) break
          for (const frame of consumeSse(decoder.decode(value, { stream: true }), state, 64_000)) {
            if (frame.event === 'ready') {
              // A reconnect may have hidden changes made while the stream was down.
              if (connectedBefore) callbacks.onResync()
              connectedBefore = true
              delay = 1000
            } else if (frame.event === 'knowledge_changed') {
              const paths = changedPaths(frame.data)
              if (paths) callbacks.onChanged(paths)
              else callbacks.onResync()
            } else if (frame.event === 'sync_status') {
              try {
                const status = JSON.parse(frame.data) as SyncStatus
                if (status && typeof status === 'object' && 'type' in status) callbacks.onSyncStatus?.(status)
                else callbacks.onResync()
              } catch { callbacks.onResync() }
            }
          }
        }
      } finally {
        reader.releaseLock()
      }
    } catch {
      if (signal.aborted) return
    }
    if (signal.aborted) break
    await pause(delay, signal)
    delay = Math.min(delay * 2, 30_000)
  }
}

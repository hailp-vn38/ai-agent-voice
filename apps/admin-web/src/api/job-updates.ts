import { apiUrl } from './client'
import { consumeSse, createSseParserState } from './sse'
import { readAdminToken } from '@/stores/auth'

export interface JobChangeCallbacks {
  onReady: () => void
  onChanged: (jobId: string) => void
  onResync: () => void
}

function pause(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise(resolve => {
    if (signal.aborted) return resolve()
    const finish = () => { clearTimeout(timer); signal.removeEventListener('abort', finish); resolve() }
    const timer = setTimeout(finish, ms)
    signal.addEventListener('abort', finish, { once: true })
  })
}

/** Subscribe to ephemeral change hints; REST remains the authoritative read. */
export async function listenJobChanges(callbacks: JobChangeCallbacks, signal: AbortSignal): Promise<void> {
  const connected = { value: false }
  let delay = 1000
  while (!signal.aborted) {
    try {
      const token = readAdminToken()
      const response = await fetch(apiUrl('/api/admin/jobs/stream'), {
        headers: {
          Accept: 'text/event-stream',
          ...(token ? { Authorization: `Bearer ${token}` } : {}),
        },
        cache: 'no-store',
        signal,
      })
      if (response.status === 401 || response.status === 403) {
        callbacks.onResync()
        return
      }
      if (!response.ok || !response.body ||
        !response.headers.get('content-type')?.startsWith('text/event-stream')) {
        throw new Error('job_sse_unavailable')
      }
      const reader = response.body.getReader()
      const decoder = new TextDecoder()
      const state = createSseParserState()
      try {
        while (!signal.aborted) {
          const { done, value } = await reader.read()
          if (done) break
          const frames = consumeSse(decoder.decode(value, { stream: true }), state, 8192)
          for (const frame of frames) {
            if (frame.event === 'ready') {
              if (connected.value) callbacks.onResync()
              else callbacks.onReady()
              connected.value = true
            } else if (frame.event === 'resync_required') {
              callbacks.onResync()
            } else if (frame.event === 'job_changed') {
              try {
                const payload: unknown = JSON.parse(frame.data)
                if (payload && typeof payload === 'object' && 'job_id' in payload &&
                  typeof payload.job_id === 'string') callbacks.onChanged(payload.job_id)
                else callbacks.onResync()
              } catch {
                callbacks.onResync()
              }
            }
          }
          if (connected.value) delay = 1000
        }
      } finally {
        reader.releaseLock()
      }
    } catch {
      if (signal.aborted) return
    }
    if (signal.aborted) break
    // A network gap may have hidden commits even if no broadcast event was lost.
    callbacks.onResync()
    await pause(delay, signal)
    delay = Math.min(delay * 2, 30_000)
  }
}

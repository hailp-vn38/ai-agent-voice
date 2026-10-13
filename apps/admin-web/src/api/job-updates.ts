import { apiUrl } from './client'
import { readAdminToken } from '@/stores/auth'

export interface JobChangeCallbacks {
  onReady: () => void
  onChanged: (jobId: string) => void
  onResync: () => void
}

function consumeSse(
  chunk: string,
  state: { buffer: string; event: string; data: string[]; lastCr: boolean },
  callbacks: JobChangeCallbacks,
  connected: { value: boolean },
) {
  let text = chunk
  if (state.lastCr && text.startsWith('\n')) text = text.slice(1)
  state.lastCr = text.endsWith('\r')
  state.buffer += text.replace(/\r\n?/g, '\n')
  if (state.buffer.length > 8192) throw new Error('job_sse_frame_too_large')
  let newline: number
  while ((newline = state.buffer.indexOf('\n')) !== -1) {
    const line = state.buffer.slice(0, newline)
    state.buffer = state.buffer.slice(newline + 1)
    if (line === '') {
      if (state.event === 'ready') {
        if (connected.value) callbacks.onResync()
        else callbacks.onReady()
        connected.value = true
      } else if (state.event === 'resync_required') {
        callbacks.onResync()
      } else if (state.event === 'job_changed') {
        try {
          const payload: unknown = JSON.parse(state.data.join('\n'))
          if (payload && typeof payload === 'object' && 'job_id' in payload &&
            typeof payload.job_id === 'string') callbacks.onChanged(payload.job_id)
          else callbacks.onResync()
        } catch {
          callbacks.onResync()
        }
      }
      state.event = ''
      state.data = []
    } else if (line.startsWith('event:')) {
      state.event = line.slice(6).trimStart()
    } else if (line.startsWith('data:')) {
      state.data.push(line.slice(5).trimStart())
      if (state.data.reduce((sum, part) => sum + part.length, 0) > 8192) {
        throw new Error('job_sse_frame_too_large')
      }
    }
  }
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
      const state = { buffer: '', event: '', data: [] as string[], lastCr: false }
      try {
        while (!signal.aborted) {
          const { done, value } = await reader.read()
          if (done) break
          consumeSse(decoder.decode(value, { stream: true }), state, callbacks, connected)
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

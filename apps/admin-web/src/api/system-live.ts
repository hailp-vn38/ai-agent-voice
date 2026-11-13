import { readAdminWebToken } from '@/stores/auth'

import { apiUrl } from './client'
import { handleSessionResponse } from './session'
import { consumeSse, createSseParserState } from './sse'
import type { SystemLiveSnapshot, SystemResources, SystemStatus } from './types/system'

export type SystemLiveTransport = (
  onSnapshot: (snapshot: SystemLiveSnapshot) => void,
  signal: AbortSignal,
) => Promise<'ended' | 'unauthorized'>

function isSystemResources(value: unknown): value is SystemResources {
  if (!value || typeof value !== 'object') return false
  const resource = value as Partial<SystemResources>
  return typeof resource.sampled_at === 'string' &&
    typeof resource.sample_interval_ms === 'number' &&
    Boolean(resource.system && resource.process && resource.storage)
}

function isSystemStatus(value: unknown): value is SystemStatus {
  if (!value || typeof value !== 'object') return false
  const status = value as Partial<SystemStatus>
  return typeof status.status === 'string' &&
    typeof status.version === 'string' &&
    typeof status.uptime_seconds === 'number' &&
    Boolean(status.database && status.providers && status.sessions)
}

function parseSnapshot(data: string): SystemLiveSnapshot {
  let value: unknown
  try {
    value = JSON.parse(data)
  } catch {
    throw new Error('invalid_system_live_snapshot')
  }
  if (!value || typeof value !== 'object') throw new Error('invalid_system_live_snapshot')
  const snapshot = value as Partial<SystemLiveSnapshot>
  const status = snapshot.status
  if (snapshot.v !== 1 ||
    !(snapshot.resources === null || isSystemResources(snapshot.resources)) ||
    !status ||
    !(status.checked_at === null || typeof status.checked_at === 'string') ||
    typeof status.refresh_interval_ms !== 'number' ||
    !(status.summary === null || isSystemStatus(status.summary))) {
    throw new Error('invalid_system_live_snapshot')
  }
  return snapshot as SystemLiveSnapshot
}

/** Opens one authenticated System Live Stream and returns when the server closes it. */
export const listenSystemLive: SystemLiveTransport = async (onSnapshot, signal) => {
  try {
    const token = readAdminWebToken()
    const response = await fetch(apiUrl('/api/admin/system/live'), {
      headers: {
        Accept: 'text/event-stream',
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
      },
      cache: 'no-store',
      signal,
    })
    if (response.status === 401 || response.status === 403) {
      await handleSessionResponse(response.clone())
      return 'unauthorized'
    }
    if (!response.ok || !response.body ||
      !response.headers.get('content-type')?.startsWith('text/event-stream')) {
      throw new Error('system_live_sse_unavailable')
    }

    const reader = response.body.getReader()
    const decoder = new TextDecoder()
    const parser = createSseParserState()
    try {
      while (!signal.aborted) {
        const { done, value } = await reader.read()
        if (done) return 'ended'
        const frames = consumeSse(decoder.decode(value, { stream: true }), parser, 16_384)
        for (const frame of frames) {
          if (frame.event === 'snapshot') onSnapshot(parseSnapshot(frame.data))
        }
      }
      return 'ended'
    } finally {
      reader.releaseLock()
    }
  } catch (error) {
    if (signal.aborted) return 'ended'
    throw error
  }
}

import { afterEach, describe, expect, it, vi } from 'vitest'

import { listenSystemLive } from './system-live'

const session = vi.hoisted(() => ({ handleSessionResponse: vi.fn() }))
vi.mock('./session', () => session)
vi.mock('@/stores/auth', () => ({ readAdminWebToken: () => 'test-admin-token' }))

const snapshot = {
  v: 1,
  resources: null,
  status: { checked_at: null, refresh_interval_ms: 15_000, summary: null },
} as const

afterEach(() => {
  vi.unstubAllGlobals()
  vi.clearAllMocks()
})

describe('System Live SSE transport', () => {
  it('sends the bearer and delivers complete snapshot frames', async () => {
    const encoder = new TextEncoder()
    const frame = `event: snapshot\r\ndata: ${JSON.stringify(snapshot)}\r\n\r\n`
    const responseBody = encoder.encode(frame)
    const split = responseBody.indexOf(0x0a) + 1
    const fetchMock = vi.fn().mockResolvedValue(new Response(new ReadableStream<Uint8Array>({
      start(stream) {
        stream.enqueue(responseBody.slice(0, split))
        stream.enqueue(responseBody.slice(split))
        stream.close()
      },
    }), { headers: { 'Content-Type': 'text/event-stream; charset=utf-8' } }))
    vi.stubGlobal('fetch', fetchMock)
    const received: unknown[] = []

    const result = await listenSystemLive(value => received.push(value), new AbortController().signal)

    expect(result).toBe('ended')
    expect(received).toEqual([snapshot])
    expect(fetchMock.mock.calls[0]?.[0]).toContain('/api/admin/system/live')
    const init = fetchMock.mock.calls[0]?.[1] as RequestInit
    expect(new Headers(init.headers).get('accept')).toBe('text/event-stream')
    expect(new Headers(init.headers).get('authorization')).toBe('Bearer test-admin-token')
    expect(init.cache).toBe('no-store')
  })

  it.each([401, 403])('handles authorization status %s and returns unauthorized', async status => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('', { status })))
    session.handleSessionResponse.mockResolvedValue(true)

    await expect(listenSystemLive(vi.fn(), new AbortController().signal)).resolves.toBe('unauthorized')
    expect(session.handleSessionResponse).toHaveBeenCalledTimes(1)
  })

  it('rejects invalid stream responses and malformed snapshots', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('{}', { status: 200 })))
    await expect(listenSystemLive(vi.fn(), new AbortController().signal)).rejects.toThrow('system_live_sse_unavailable')

    const invalidFrame = new Response('event: snapshot\ndata: {"v":2}\n\n', {
      headers: { 'Content-Type': 'text/event-stream' },
    })
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(invalidFrame))
    await expect(listenSystemLive(vi.fn(), new AbortController().signal)).rejects.toThrow('invalid_system_live_snapshot')
  })
})

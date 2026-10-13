import { afterEach, describe, expect, it, vi } from 'vitest'
import { listenJobChanges } from './job-updates'

vi.mock('@/stores/auth', () => ({ readAdminToken: () => 'test-admin-token' }))

afterEach(() => vi.unstubAllGlobals())

describe('Job SSE listener', () => {
  it('uses bearer authorization and handles UTF-8 and CRLF fragments', async () => {
    const abort = new AbortController()
    const received: string[] = []
    const encoder = new TextEncoder()
    const chunks = encoder.encode(
      'event: ready\r\ndata: {}\r\n\r\n' +
      'event: job_changed\r\ndata: {"job_id":"job-ä"}\r\n\r\n',
    )
    const split = chunks.indexOf(0xc3) + 1
    const fetch = vi.fn().mockResolvedValue(new Response(new ReadableStream({
      start(stream) {
        stream.enqueue(chunks.slice(0, split))
        stream.enqueue(chunks.slice(split))
        stream.close()
      },
    }), { headers: { 'Content-Type': 'text/event-stream' } }))
    vi.stubGlobal('fetch', fetch)
    await listenJobChanges({
      onReady: () => received.push('ready'),
      onChanged: id => { received.push(id); abort.abort() },
      onResync: () => received.push('resync'),
    }, abort.signal)
    expect(received).toEqual(['ready', 'job-ä'])
    expect(fetch.mock.calls[0]?.[0]).toContain('/api/admin/jobs/stream')
    const init = fetch.mock.calls[0]?.[1] as RequestInit
    expect(new Headers(init.headers).get('authorization')).toBe('Bearer test-admin-token')
  })

  it('does not reconnect after an authorization rejection', async () => {
    const fetch = vi.fn().mockResolvedValue(new Response('', { status: 401 }))
    vi.stubGlobal('fetch', fetch)
    const callbacks = {
      onReady: vi.fn(),
      onChanged: vi.fn(),
      onResync: vi.fn(),
    }
    await listenJobChanges(callbacks, new AbortController().signal)
    expect(fetch).toHaveBeenCalledTimes(1)
    expect(callbacks.onReady).not.toHaveBeenCalled()
  })
})

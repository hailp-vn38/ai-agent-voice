import { afterEach, describe, expect, it, vi } from 'vitest'
import { listenKnowledgeChanges } from './knowledge-updates'

vi.mock('@/stores/auth', () => ({ readAdminToken: () => 'test-admin-token' }))

afterEach(() => vi.unstubAllGlobals())

function sseResponse(text: string) {
  return new Response(new ReadableStream<Uint8Array>({
    start(stream) {
      stream.enqueue(new TextEncoder().encode(text))
      stream.close()
    },
  }), { headers: { 'Content-Type': 'text/event-stream' } })
}

describe('Knowledge SSE listener', () => {
  it('reports pushed paths with bearer authorization', async () => {
    const abort = new AbortController()
    const received: string[][] = []
    const fetch = vi.fn().mockResolvedValue(sseResponse(
      'event: ready\r\ndata: {}\r\n\r\n' +
      'event: knowledge_changed\r\ndata: {"paths":["wiki/cà-chua.md"],"truncated":false}\r\n\r\n',
    ))
    vi.stubGlobal('fetch', fetch)
    await listenKnowledgeChanges({
      onChanged: paths => { received.push(paths); abort.abort() },
      onResync: () => received.push(['resync']),
    }, abort.signal)
    expect(received).toEqual([['wiki/cà-chua.md']])
    expect(fetch.mock.calls[0]?.[0]).toContain('/api/admin/knowledge/events')
    const init = fetch.mock.calls[0]?.[1] as RequestInit
    expect(new Headers(init.headers).get('authorization')).toBe('Bearer test-admin-token')
  })

  it('forwards sync_status events to the sync adapter callback', async () => {
    const abort = new AbortController()
    const statuses: unknown[] = []
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(sseResponse(
      'event: ready\ndata: {}\n\n' +
      'event: sync_status\ndata: {"type":"error","kind":"auth","message":"safe"}\n\n',
    )))
    await listenKnowledgeChanges({
      onChanged: () => {}, onResync: () => {},
      onSyncStatus: status => { statuses.push(status); abort.abort() },
    }, abort.signal)
    expect(statuses).toEqual([{ type: 'error', kind: 'auth', message: 'safe' }])
  })

  it('reports no paths for a truncated change and resyncs after a reconnect', async () => {
    vi.useFakeTimers()
    try {
      const abort = new AbortController()
      const received: string[] = []
      const fetch = vi.fn()
        .mockResolvedValueOnce(sseResponse(
          'event: ready\ndata: {}\n\nevent: knowledge_changed\ndata: {"paths":["wiki/a.md"],"truncated":true}\n\n',
        ))
        .mockResolvedValueOnce(sseResponse('event: ready\ndata: {}\n\n'))
      vi.stubGlobal('fetch', fetch)
      const done = listenKnowledgeChanges({
        onChanged: paths => received.push(`changed:${paths.join(',')}`),
        onResync: () => { received.push('resync'); abort.abort() },
      }, abort.signal)
      await vi.runAllTimersAsync()
      await done
      expect(received).toEqual(['changed:', 'resync'])
      expect(fetch).toHaveBeenCalledTimes(2)
    } finally {
      vi.useRealTimers()
    }
  })

  it('does not reconnect after an authorization rejection', async () => {
    const fetch = vi.fn().mockResolvedValue(new Response('', { status: 401 }))
    vi.stubGlobal('fetch', fetch)
    const callbacks = { onChanged: vi.fn(), onResync: vi.fn() }
    await listenKnowledgeChanges(callbacks, new AbortController().signal)
    expect(fetch).toHaveBeenCalledTimes(1)
    expect(callbacks.onChanged).not.toHaveBeenCalled()
  })
})

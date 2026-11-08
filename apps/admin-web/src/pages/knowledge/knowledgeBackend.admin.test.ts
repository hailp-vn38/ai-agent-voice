import { describe, expect, it } from 'vitest'

import { HttpKnowledgeBackend, MemoryKnowledgeBackend, type Change, type AdminKnowledgeBackend, type KnowledgePage } from './knowledgeBackend'

function makeHttpServer(): AdminKnowledgeBackend {
  const pages = new Map<string, KnowledgePage>([['wiki/gardening.md', {
    id: 'wiki/gardening.md', path: 'wiki/gardening.md', title: 'Gardening', body: 'Trồng cà chua trong nhà kính.', revision: 1,
  }]])
  const request = async <T>(path: string, init: RequestInit = {}): Promise<T> => {
    const url = new URL(path, 'http://wiki.test')
    const method = init.method ?? 'GET'
    if (url.pathname === '/api/admin/knowledge/tree') {
      return { items: [...pages.values()].map(page => ({
        type: 'file', space: url.searchParams.get('space'), path: page.path,
        name: page.path?.split('/').at(-1), editable: true, id: page.id, revision: page.revision,
      })) } as T
    }
    if (url.pathname === '/api/admin/knowledge/index') {
      return { health: 'healthy', document_count: pages.size, last_complete_scan_at: null } as T
    }
    if (url.pathname === '/api/admin/knowledge/pages' && url.searchParams.has('q')) {
      const q = url.searchParams.get('q')!.normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase()
      return { items: [...pages.values()].filter(page => page.body.normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase().includes(q))
        .map(page => ({ id: page.id, path: page.path, title: page.title, revision: page.revision, excerpt: page.body })) } as T
    }
    const prefix = '/api/admin/knowledge/pages/'
    if (url.pathname.startsWith(prefix)) {
      const pathId = decodeURIComponent(url.pathname.slice(prefix.length))
      if (method === 'PUT') {
        const body = JSON.parse(String(init.body)) as { body: string; expected_revision?: number }
        const previous = pages.get(pathId)
        if (previous && body.expected_revision !== undefined && body.expected_revision !== previous.revision) {
          throw new Error('conflict')
        }
        const page = { ...previous!, body: body.body, revision: (previous?.revision ?? 0) + 1 }
        pages.set(pathId, page)
        return page as T
      }
      return pages.get(pathId) as T
    }
    throw new Error(`Unexpected fake server request: ${method} ${path}`)
  }
  return new HttpKnowledgeBackend(request, async () => {}).admin!
}

const adapters: Array<[string, () => AdminKnowledgeBackend]> = [
  ['MemoryAdminKnowledgeBackend', () => new MemoryKnowledgeBackend({
    pages: [{ id: 'wiki/gardening.md', path: 'wiki/gardening.md', title: 'Gardening', body: 'Trồng cà chua trong nhà kính.', revision: 1 }],
  }).admin!],
  ['HttpKnowledgeBackend', makeHttpServer],
]

for (const [name, createBackend] of adapters) {
  describe(`${name} contract`, () => {
    it('reads, searches without Vietnamese diacritics, and applies a revisioned write', async () => {
      const backend = createBackend()
      expect((await backend.tree('wiki')).items.filter(item => item.type === 'file').map(item => item.path)).toEqual(['wiki/gardening.md'])
      expect((await backend.read('wiki/gardening.md')).body).toContain('cà chua')
      expect((await backend.search('ca chua')).items.map(item => item.path)).toEqual(['wiki/gardening.md'])

      await backend.apply({ type: 'write', path: 'wiki/gardening.md', body: 'Updated', expect: 1 })
      expect((await backend.read('wiki/gardening.md')).body).toBe('Updated')
      expect((await backend.search('updated')).items.map(item => item.path)).toEqual(['wiki/gardening.md'])
    })
  })
}

describe('MemoryKnowledgeBackend', () => {
  it('supports the shared read and search contract over a wiki page', async () => {
    const backend = new MemoryKnowledgeBackend({
      pages: [{ id: 'wiki/gardening.md', path: 'wiki/gardening.md', title: 'Gardening', body: 'Trồng cà chua trong nhà kính.', revision: 1 }],
    }).admin!

    expect((await backend.tree('wiki')).items.filter(item => item.type === 'file').map(item => item.path)).toEqual(['wiki/gardening.md'])
    expect((await backend.read('wiki/gardening.md')).body).toContain('cà chua')
    expect((await backend.search('ca chua')).items.map(item => item.path)).toEqual(['wiki/gardening.md'])
  })

  it('turns pushed server changes into external events and stops listening on unsubscribe', async () => {
    let callbacks: import('@/api/knowledge-updates').KnowledgeChangeCallbacks | undefined
    let signal: AbortSignal | undefined
    const backend = new HttpKnowledgeBackend(async <T>() => ({}) as T, async (next, nextSignal) => {
      callbacks = next
      signal = nextSignal
    }).admin!
    const events: unknown[] = []
    const stop = backend.onChange(event => events.push(event))

    callbacks!.onChanged(['wiki/gardening.md'])
    callbacks!.onResync()
    stop()

    expect(events).toEqual([
      { type: 'external', origin: 'server', paths: ['wiki/gardening.md'] },
      { type: 'external', origin: 'server', paths: [] },
    ])
    expect(signal!.aborted).toBe(true)
  })

  it('publishes a change after apply and returns a typed conflict for a stale revision', async () => {
    const backend = new MemoryKnowledgeBackend({ pages: [
      { id: 'wiki/gardening.md', path: 'wiki/gardening.md', title: 'Gardening', body: 'Original', revision: 1 },
    ] }).admin!
    const events: unknown[] = []
    backend.onChange(event => events.push(event))

    const change: Change = { type: 'write', path: 'wiki/gardening.md', body: 'Updated', expect: '1' }
    await backend.apply(change)

    expect((await backend.read('wiki/gardening.md')).body).toBe('Updated')
    expect(events).toEqual([{ type: 'changed', paths: ['wiki/gardening.md'], origin: 'ui' }])
    await expect(backend.apply({ ...change, body: 'Stale', expect: 'old-revision' }))
      .rejects.toMatchObject({ kind: 'conflict', current: '2' })
    expect((await backend.read('wiki/gardening.md')).body).toBe('Updated')
  })
})

import { describe, expect, it } from 'vitest'

import {
  docPath,
  HttpKnowledgeBackend,
  MemoryKnowledgeBackend,
  type KnowledgeBackend,
} from './knowledgeBackend'

function makeHttpBackend(): KnowledgeBackend {
  let storedBody = 'first\nsecond\nthird'
  let revision = 1
  const request = async <T>(path: string, init: RequestInit = {}): Promise<T> => {
    const url = new URL(path, 'http://wiki.test')
    if (url.pathname === '/api/admin/knowledge/tree') return { items: [
      { type: 'folder', space: 'wiki', path: 'wiki/garden', name: 'garden', editable: true },
      { type: 'file', space: 'wiki', path: 'wiki/garden/tomatoes.md', name: 'tomatoes.md', id: 'tomatoes', revision: 1, editable: true },
    ] } as T
    if (url.pathname === '/api/admin/knowledge/index') return { health: 'ready', document_count: 1, sources_count: 0, wiki_count: 1 } as T
    if (url.pathname === '/api/admin/knowledge/graph') return { nodes: [
      { id: 'tomatoes', path: 'wiki/garden/tomatoes.md', title: 'Tomatoes', label: 'Tomatoes', inbound: 2, outbound: 1 },
    ], edges: [], unresolved: [] } as T
    if (url.pathname.startsWith('/api/admin/knowledge/pages/')) {
      if (init.method === 'PUT') {
        storedBody = (JSON.parse(String(init.body)) as { body: string }).body
        revision += 1
      }
      return { id: 'tomatoes', path: 'wiki/garden/tomatoes.md', title: 'Tomatoes',
        body: storedBody, revision } as T
    }
    if (url.pathname === '/api/admin/knowledge/pages' && url.searchParams.has('q')) return { items: [
      { id: 'tomatoes', path: 'wiki/garden/tomatoes.md', title: 'Tomatoes', revision: 1, excerpt: 'second' },
    ] } as T
    throw new Error(`Unexpected fake server request: ${path}`)
  }
  return new HttpKnowledgeBackend(request)
}

const adapters: Array<[string, () => KnowledgeBackend]> = [
  ['MemoryKnowledgeBackend', () => new MemoryKnowledgeBackend({ pages: [
    { id: 'tomatoes', path: 'wiki/garden/tomatoes.md', title: 'Tomatoes', body: 'first\nsecond\nthird', revision: 1 },
  ], graph: { nodes: [{ path: 'wiki/garden/tomatoes.md', title: 'Tomatoes', label: 'Tomatoes', inbound: 2, outbound: 1 }], edges: [], unresolved: [] } })],
  ['HttpKnowledgeBackend', makeHttpBackend],
]

for (const [name, createBackend] of adapters) {
  describe(`${name} Vault contract`, () => {
    it('uses validated paths and Vault-shaped tree, read window, search, and write results', async () => {
      const backend = createBackend()
      const path = docPath('wiki', 'garden/tomatoes.md')

      const tree = await backend.tree('wiki')
      expect(tree.children[0]).toMatchObject({ name: 'garden', is_directory: true })
      expect(tree.children[0]?.children[0]).toMatchObject({ path: 'wiki/garden/tomatoes.md', is_directory: false })
      expect((await backend.read(path, { from_line: 1, max_lines: 1 })).content).toBe('second')
      expect((await backend.search('Tomatoes')).map(hit => hit.path)).toEqual(['wiki/garden/tomatoes.md'])
      expect((await backend.status()).wiki_pages).toBe(1)
      expect((await backend.graph()).nodes).toEqual([{ path: 'wiki/garden/tomatoes.md', title: 'Tomatoes', label: 'Tomatoes', inbound: 2, outbound: 1 }])

      const applied = await backend.apply({ type: 'write', path, content: 'updated', expect: '1' })
      expect(applied.paths).toEqual(['wiki/garden/tomatoes.md'])
      expect((await backend.read(path)).content).toBe('updated')
    })
  })
}

it('rejects paths outside the vault scope before any adapter call', async () => {
  expect(() => docPath('wiki', '../secret.md')).toThrow()
  expect(() => docPath('wiki', '/absolute.md')).toThrow()
})

it('translates Vault paths back to legacy Admin page ids for reads', async () => {
  const pageRoutes: string[] = []
  const backend = new HttpKnowledgeBackend(async <T>(path: string): Promise<T> => {
    const url = new URL(path, 'http://wiki.test')
    if (url.pathname === '/api/admin/knowledge/tree') return { items: [
      { type: 'file', space: 'wiki', path: 'wiki/garden/tomatoes.md', name: 'tomatoes.md', id: 'tomatoes', revision: 1, editable: true },
    ] } as T
    if (url.pathname.startsWith('/api/admin/knowledge/pages/')) {
      pageRoutes.push(url.pathname)
      return { id: 'tomatoes', title: 'Tomatoes', body: '# Tomatoes', revision: 1 } as T
    }
    throw new Error(`Unexpected fake server request: ${path}`)
  })

  await backend.tree('wiki')
  expect((await backend.read(docPath('wiki', 'garden/tomatoes.md'))).title).toBe('Tomatoes')
  expect(pageRoutes).toEqual(['/api/admin/knowledge/pages/tomatoes'])
})

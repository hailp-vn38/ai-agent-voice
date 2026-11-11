import { describe, expect, it } from 'vitest'
import { MemoryKnowledgeBackend, docPath } from './knowledgeBackend'

describe('MemoryKnowledgeBackend', () => {
  it('supports the shared browse, read, search, and write contract in memory', async () => {
    const backend = new MemoryKnowledgeBackend()
    const path = docPath('wiki', 'garden/tomatoes.md')

    await backend.apply({ type: 'write', path, content: '# Tomatoes\n\nGrow in full sun.' })

    const tree = await backend.tree('wiki')
    expect(tree.children[0]).toMatchObject({ name: 'garden', is_directory: true })
    await expect(backend.read(path)).resolves.toMatchObject({ title: 'tomatoes.md', content: '# Tomatoes\n\nGrow in full sun.' })
    await expect(backend.search('full sun')).resolves.toEqual(expect.arrayContaining([
      expect.objectContaining({ path: 'wiki/garden/tomatoes.md', title: 'tomatoes.md' }),
    ]))
  })
})

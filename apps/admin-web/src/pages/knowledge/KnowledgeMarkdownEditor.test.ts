import { describe, expect, it } from 'vitest'
import { linePrefixEdit, listContinuation, textStats, wikiLinkQuery, wrapEdit } from '@vela/knowledge-ui/components/KnowledgeMarkdownEditor.vue'

const run = (value: string, e: { text: string; start: number; end: number }) =>
  value.slice(0, e.start) + e.text + value.slice(e.end)

describe('markdown editor edits', () => {
  it('wraps and unwraps a selection', () => {
    const wrapped = wrapEdit('a word b', 2, 6, '**')
    expect(run('a word b', wrapped)).toBe('a **word** b')
    const back = wrapEdit('a **word** b', 4, 8, '**')
    expect(run('a **word** b', back)).toBe('a word b')
  })
  it('toggles line prefixes and numbers ordered lists', () => {
    expect(run('one\ntwo', linePrefixEdit('one\ntwo', 0, 7, '1. '))).toBe('1. one\n2. two')
    expect(run('# T', linePrefixEdit('# T', 0, 0, '## '))).toBe('## T')
    expect(run('- a', linePrefixEdit('- a', 0, 0, '- '))).toBe('a')
  })
  it('continues and ends lists on Enter', () => {
    expect(run('1. a', listContinuation('1. a', 4)!)).toBe('1. a\n2. ')
    expect(run('- [x] a', listContinuation('- [x] a', 7)!)).toBe('- [x] a\n- [ ] ')
    expect(run('- a\n- ', listContinuation('- a\n- ', 6)!)).toBe('- a\n')
    expect(listContinuation('plain', 5)).toBeNull()
  })
  it('finds an open wiki link query', () => {
    expect(wikiLinkQuery('see [[pro', 9)).toEqual({ from: 4, query: 'pro' })
    expect(wikiLinkQuery('see [[done]] x', 14)).toBeNull()
  })
  it('counts words', () => {
    expect(textStats('Xin chào thế giới').words).toBe(4)
  })
})

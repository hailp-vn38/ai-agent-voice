import { expect, it } from 'vitest'
import { markdownOutline, renderMarkdown } from './markdown'

it('renders Mermaid fences as escaped, detectable diagram source', () => {
  const body = '## Flow\n\n' +
    '~~~mermaid\nflowchart TD\n  A[Unsafe <script>alert(1)</script>] --> B\n~~~\n'
  const rendered = renderMarkdown(body)
  expect(rendered).toContain('<pre class="knowledge-mermaid" data-mermaid-source="true">')
  expect(rendered).toContain('&lt;script&gt;alert(1)&lt;/script&gt;')
  expect(rendered).not.toContain('<script>')
  expect(markdownOutline(body).map(item => item.title)).toEqual(['Flow'])
})

it('keeps ordinary fenced code as code, not a Mermaid diagram', () => {
  const rendered = renderMarkdown('~~~rust\nfn main() {}\n~~~')
  expect(rendered).toContain('language-rust')
  expect(rendered).not.toContain('knowledge-mermaid')
})

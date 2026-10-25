import { expect, it } from 'vitest'
import { normalizeMediaMarkdownLinks } from '@/lib/markdown'
import { markdownOutline, renderMarkdown } from './markdown'
import { renderMarkdown as renderReadingMarkdown } from './readingMarkdown'

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

it('normalizes and renders image and video links with ![link] syntax', () => {
  const raw = [
    '- [Ảnh minh họa](https://cdn.example.com/photo.png)',
    '- [Video YouTube](https://www.youtube.com/watch?v=uY13C-jwXFk)',
    '- [Clip MP4](https://cdn.example.com/demo.mp4)',
    '- [Bài viết web](https://example.com/bai-viet.html)',
  ].join('\n')

  const normalized = normalizeMediaMarkdownLinks(raw)
  expect(normalized).toContain('![Ảnh minh họa](https://cdn.example.com/photo.png)')
  expect(normalized).toContain('![Video YouTube](https://www.youtube.com/watch?v=uY13C-jwXFk)')
  expect(normalized).toContain('![Clip MP4](https://cdn.example.com/demo.mp4)')
  expect(normalized).toContain('- [Bài viết web](https://example.com/bai-viet.html)')

  for (const html of [renderMarkdown(normalized), renderReadingMarkdown(normalized)]) {
    expect(html).toContain('<img alt="Ảnh minh họa" src="https://cdn.example.com/photo.png" loading="lazy">')
    expect(html).toContain('data-media-embed="video"')
    expect(html).toContain('<video controls preload="metadata" src="https://cdn.example.com/demo.mp4">')
    expect(html).not.toContain('!<a ')
  }
})

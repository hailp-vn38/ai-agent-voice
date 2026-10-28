import { expect, it } from 'vitest'
import {
  markdownOutline,
  normalizeMediaMarkdownLinks,
  parseMarkdownProperties,
  renderMarkdown,
} from '@/lib/markdown'

it('renders Mermaid fences as escaped, detectable diagram source', () => {
  const body = '## Flow\n\n' +
    '~~~mermaid\nflowchart TD\n  A[Unsafe <script>alert(1)</script>] --> B\n~~~\n'
  const rendered = renderMarkdown(body)
  expect(rendered).toContain('<pre class="knowledge-mermaid" data-mermaid-source="true">')
  expect(rendered).toContain('&lt;script&gt;alert(1)&lt;/script&gt;')
  expect(rendered).not.toContain('<script>')
  expect(markdownOutline(body).map(item => item.title)).toEqual(['Flow'])
})

it('renders trading-chart fences as escaped, detectable chart source without affecting WikiLinks or Mermaid', () => {
  const body = [
    '## Bullish Order Block',
    '',
    'Xem thêm [[order-block-guide|Order Block Guide]].',
    '',
    '```trading-chart',
    '{"src":"charts/bullish-ob-fvg-example.chart.json","note":"<img onerror=alert(1)>"}',
    '```',
  ].join('\n')
  const rendered = renderMarkdown(body)
  expect(rendered).toContain('<pre class="knowledge-trading-chart" data-trading-chart-source="true">')
  expect(rendered).toContain('{&quot;src&quot;:&quot;charts/bullish-ob-fvg-example.chart.json&quot;,&quot;note&quot;:&quot;&lt;img onerror=alert(1)&gt;&quot;}')
  expect(rendered).not.toContain('<img onerror=alert(1)>')
  expect(rendered).toContain('data-wikilink="order-block-guide"')
  expect(markdownOutline(body).map(item => item.title)).toEqual(['Bullish Order Block'])
})

it('keeps ordinary fenced code as code, not a Mermaid diagram or trading chart', () => {
  const rendered = renderMarkdown('~~~rust\nfn main() {}\n~~~')
  expect(rendered).toContain('language-rust')
  expect(rendered).not.toContain('knowledge-mermaid')
  expect(rendered).not.toContain('knowledge-trading-chart')
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

  const html = renderMarkdown(normalized)
  expect(html).toContain('<img alt="Ảnh minh họa" src="https://cdn.example.com/photo.png" loading="lazy">')
  expect(html).toContain('data-media-embed="video"')
  expect(html).toContain('<video controls preload="metadata" src="https://cdn.example.com/demo.mp4">')
  expect(html).not.toContain('!<a ')
})

it('parses YAML frontmatter into structured properties and strips it from rendered Markdown', () => {
  const doc = [
    '---',
    'title: "Đón không khí lạnh, miền Bắc giảm mấy độ?"',
    'category: "source_summary"',
    'summary: "Tin ngắn cho biết không khí lạnh tác động miền Bắc ngày 21–22/10/2026."',
    'source_file: "don-khong-khi-lanh-mien-bac-giam-may-do.md"',
    'source_count: 1',
    'sources:',
    '  - "don-khong-khi-lanh-mien-bac-giam-may-do.md"',
    'tags:',
    '  - "wiki/source"',
    '  - "topic/weather/cold-air"',
    'updated_at: "2026-10-24T17:17:57.633104+00:00"',
    'qmd:',
    '  metadata:',
    '    category: "source_summary"',
    '    tags:',
    '      - "wiki/source"',
    '---',
    '',
    '## Diễn biến chính',
    '',
    'Nội dung bài viết.',
  ].join('\n')

  const props = parseMarkdownProperties(doc)
  expect(props.map(p => p.key)).toEqual(['category', 'tags', 'sources', 'updated_at', 'summary'])
  expect(props.find(p => p.key === 'category')?.values).toEqual(['source_summary'])
  expect(props.find(p => p.key === 'tags')?.values).toEqual(['wiki/source', 'topic/weather/cold-air'])
  expect(props.find(p => p.key === 'sources')?.values).toEqual(['don-khong-khi-lanh-mien-bac-giam-may-do.md'])
  expect(props.find(p => p.key === 'updated_at')?.values).toEqual(['2026-10-24 17:17'])

  const html = renderMarkdown(doc)
  expect(html).not.toContain('source_summary')
  expect(html).not.toContain('qmd:')
  expect(html).toContain('Diễn biến chính')
})

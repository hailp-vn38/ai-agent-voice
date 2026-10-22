import { describe, expect, it } from 'vitest'
import { jobResultReport } from './result-report'

describe('Job Result presentation', () => {
  it('prefers the persisted Markdown report and renders readable report elements', () => {
    const report = jobResultReport({ markdown: '## Báo cáo\n\n- [Wiki](/knowledge?wiki_id=wiki_1)\n\n`code`\n\n```rust\nfn main() {}\n```', summary: 'old summary' })
    expect(report.kind).toBe('markdown')
    if (report.kind !== 'markdown') return
    expect(report.html).toContain('>Báo cáo</h2>')
    expect(report.html).toContain('<ul><li>')
    expect(report.html).toContain('href="/knowledge?wiki_id=wiki_1"')
    expect(report.html).toContain('<code>code</code>')
    expect(report.html).toContain('language-rust')
    expect(report.html).not.toContain('old summary')
  })
  it('escapes raw HTML and refuses executable link URLs', () => {
    const report = jobResultReport({ markdown: '<script>alert(1)</script>\n\n[x](javascript:alert(1))' })
    expect(report.kind).toBe('markdown')
    if (report.kind !== 'markdown') return
    expect(report.html).toContain('&lt;script&gt;')
    expect(report.html).not.toContain('<script>')
    expect(report.html).not.toMatch(/href="javascript:/i)
  })
  it('renders historical string summaries and keeps structured-only results as JSON', () => {
    expect(jobResultReport({ summary: '## Legacy' })).toMatchObject({ kind: 'markdown' })
    expect(jobResultReport({ summary: { count: 2 } })).toEqual({ kind: 'json', text: '{\n  "summary": {\n    "count": 2\n  }\n}' })
    expect(jobResultReport(null)).toEqual({ kind: 'empty' })
  })
})

import { renderMarkdown } from '@/lib/markdown'

export type JobReport = { kind: 'empty' } | { kind: 'markdown'; html: string } | { kind: 'json'; text: string }

export function jobResultReport(result: Record<string, unknown> | null | undefined): JobReport {
  if (!result) return { kind: 'empty' }
  const markdown = typeof result.markdown === 'string' && result.markdown.trim() ? result.markdown
    : typeof result.summary === 'string' && result.summary.trim() ? result.summary : null
  return markdown === null
    ? { kind: 'json', text: JSON.stringify(result, null, 2) }
    : { kind: 'markdown', html: renderMarkdown(markdown) }
}

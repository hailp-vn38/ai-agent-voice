/**
 * Read-only Markdown rendering for Knowledge and persisted Job reports.
 * User content is escaped before it becomes HTML; raw HTML never executes.
 */
export type MarkdownHeading = { id: string; level: number; title: string }

const escape = (value: string) => value.replace(/&/g, '&amp;').replace(/</g, '&lt;')
  .replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#39;')

const slug = (value: string) =>
  value.trim().toLocaleLowerCase().replace(/[^\p{L}\p{N}\s-]/gu, '').replace(/\s+/g, '-').replace(/-+/g, '-') || 'section'

export function markdownOutline(markdown: string): MarkdownHeading[] {
  const used = new Map<string, number>()
  let fence: string | null = null
  const result: MarkdownHeading[] = []
  for (const line of markdown.split(/\r?\n/)) {
    const delimiter = /^\s*(\x60{3,}|~{3,})/.exec(line)?.[1]
    if (delimiter) {
      if (fence === null) fence = delimiter[0]
      else if (delimiter[0] === fence) fence = null
      continue
    }
    if (fence) continue
    const match = /^(#{1,6})\s+(.+?)\s*#*\s*$/.exec(line)
    if (!match) continue
    const title = match[2].replace(/[*_\x60]/g, '').trim()
    const base = slug(title)
    const count = used.get(base) ?? 0
    used.set(base, count + 1)
    result.push({ id: count ? base + '-' + count : base, title, level: match[1].length })
  }
  return result
}

const safeUrl = (url: string) =>
  /^(https?:\/\/|mailto:|\/(?!\/)|\.\.?\/|#)/i.test(url.trim()) ? url.trim() : null

function inline(value: string): string {
  const pattern = /(!?\[\[[^\]\r\n]+\]\]|\[([^\]\r\n]+)\]\(([^)\r\n]+)\)|!\[([^\]\r\n]*)\]\(([^)\r\n]+)\)|\x60[^\x60\r\n]+\x60|\*\*[^*\r\n]+\*\*|\*[^*\r\n]+\*|~~[^~\r\n]+~~)/g
  let output = ''
  let at = 0
  for (const match of value.matchAll(pattern)) {
    const start = match.index ?? 0
    output += escape(value.slice(at, start))
    const token = match[0]
    if (token.startsWith('[[') || token.startsWith('![[')) {
      const parts = token.slice(token.startsWith('!') ? 3 : 2, -2).split('|', 2)
      const target = parts[0].trim()
      output += target ? '<a href="#" class="knowledge-wikilink" data-wikilink="' +
        escape(target) + '">' + escape((parts[1] || target).trim()) + '</a>' : escape(token)
    } else if (token.startsWith('![')) {
      const url = safeUrl(match[5] || '')
      output += url ? '<img alt="' + escape(match[4] || '') + '" src="' + escape(url) + '" loading="lazy">' : escape(token)
    } else if (token.startsWith('[')) {
      const url = safeUrl(match[3] || '')
      output += url ? '<a href="' + escape(url) + '" rel="noopener noreferrer" target="_blank">' +
        escape(match[2] || '') + '</a>' : escape(token)
    } else if (token.startsWith('\x60')) {
      output += '<code>' + escape(token.slice(1, -1)) + '</code>'
    } else if (token.startsWith('**')) {
      output += '<strong>' + escape(token.slice(2, -2)) + '</strong>'
    } else if (token.startsWith('~~')) {
      output += '<del>' + escape(token.slice(2, -2)) + '</del>'
    } else if (token.startsWith('*')) {
      output += '<em>' + escape(token.slice(1, -1)) + '</em>'
    }
    at = start + token.length
  }
  return output + escape(value.slice(at))
}

export function renderMarkdown(markdown: string): string {
  const headings = markdownOutline(markdown)
  let headingIndex = 0
  let output = ''
  let paragraph: string[] = []
  let list: 'ul' | 'ol' | null = null
  let quote: string[] = []
  let code: string[] | null = null
  let fence: string | null = null
  let language = ''
  const flushParagraph = () => {
    if (paragraph.length) output += '<p>' + inline(paragraph.join(' ')) + '</p>'
    paragraph = []
  }
  const flushQuote = () => {
    if (quote.length) output += '<blockquote><p>' + inline(quote.join(' ')) + '</p></blockquote>'
    quote = []
  }
  const closeList = () => {
    if (list) output += '</' + list + '>'
    list = null
  }
  for (const line of markdown.split(/\r?\n/)) {
    const delimiter = /^\s*(\x60{3,}|~{3,})\s*([\w-]*)/.exec(line)
    if (code !== null) {
      if (delimiter && delimiter[1][0] === fence) {
        const text = escape(code.join('\n'))
        output += language.toLowerCase() === 'mermaid'
          ? '<pre class="knowledge-mermaid" data-mermaid-source="true">' + text + '</pre>'
          : '<pre><code class="language-' + escape(language) + '">' + text + '</code></pre>'
        code = null
        fence = null
      } else code.push(line)
      continue
    }
    if (delimiter) {
      flushParagraph(); flushQuote(); closeList()
      code = []; fence = delimiter[1][0]; language = delimiter[2] || ''
      continue
    }
    if (!line.trim()) { flushParagraph(); flushQuote(); closeList(); continue }
    const heading = /^(#{1,6})\s+(.+?)\s*#*\s*$/.exec(line)
    if (heading) {
      flushParagraph(); flushQuote(); closeList()
      const level = heading[1].length
      output += '<h' + level + ' id="' + escape(headings[headingIndex++]?.id || slug(heading[2])) +
        '">' + inline(heading[2]) + '</h' + level + '>'
      continue
    }
    if (/^\s*(-{3,}|\*{3,}|_{3,})\s*$/.test(line)) {
      flushParagraph(); flushQuote(); closeList(); output += '<hr>'; continue
    }
    const quoted = /^\s*>\s?(.*)$/.exec(line)
    if (quoted) { flushParagraph(); closeList(); quote.push(quoted[1]); continue }
    flushQuote()
    const bullet = /^\s*[-*+]\s+(.*)$/.exec(line)
    const ordered = /^\s*\d+\.\s+(.*)$/.exec(line)
    if (bullet || ordered) {
      flushParagraph()
      const type = bullet ? 'ul' : 'ol'
      if (list !== type) { closeList(); list = type; output += '<' + type + '>' }
      let text = (bullet || ordered)![1]
      const task = /^\[([ xX])\]\s+/.exec(text)
      if (task) {
        text = text.slice(task[0].length)
        output += '<li>' + (task[1].toLowerCase() === 'x' ? '☑' : '☐') + ' ' + inline(text) + '</li>'
      } else output += '<li>' + inline(text) + '</li>'
      continue
    }
    closeList()
    paragraph.push(line.trim())
  }
  flushParagraph(); flushQuote(); closeList()
  if (code !== null) output += '<pre><code>' + escape(code.join('\n')) + '</code></pre>'
  return output
}

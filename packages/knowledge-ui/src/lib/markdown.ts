/**
 * Read-only Markdown rendering for Knowledge and persisted Job reports.
 * User content is escaped before it becomes HTML; raw HTML never executes.
 */
export type MarkdownHeading = { id: string; level: number; title: string }
export type MarkdownProperty = {
  key: string
  label: string
  kind: 'badge' | 'tags' | 'code-list' | 'date' | 'text'
  values: string[]
  raw?: string
}

const FRONTMATTER_RE = /^(?:\uFEFF)?---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/

export function stripMarkdownFrontmatter(markdown: string): string {
  const match = FRONTMATTER_RE.exec(markdown)
  return match ? markdown.slice(match[0].length) : markdown
}

function unquoteYaml(value: string): string {
  const trimmed = value.trim()
  if (
    (trimmed.startsWith('"') && trimmed.endsWith('"')) ||
    (trimmed.startsWith("'") && trimmed.endsWith("'"))
  ) {
    return trimmed.slice(1, -1).replace(/\\"/g, '"').replace(/\\\\/g, '\\').trim()
  }
  return trimmed
}

function formatPropertyDate(value: string): string {
  const match = /^(\d{4}-\d{2}-\d{2})[T\s](\d{2}:\d{2})/.exec(value)
  return match ? `${match[1]} ${match[2]}` : value
}

function formatPropertyLabel(key: string): string {
  const labels: Record<string, string> = {
    category: 'Category',
    status: 'Status',
    tags: 'Tags',
    sources: 'Sources',
    source_file: 'Source',
    updated_at: 'Updated',
    created_at: 'Created',
    summary: 'Summary',
  }
  if (labels[key]) return labels[key]
  const spaced = key.replace(/[_-]+/g, ' ').trim()
  return spaced ? spaced[0]!.toUpperCase() + spaced.slice(1) : key
}

export function parseMarkdownProperties(markdown: string): MarkdownProperty[] {
  const match = FRONTMATTER_RE.exec(markdown)
  if (!match) return []
  const entries = new Map<string, string[]>()
  let currentListKey: string | null = null

  for (const line of match[1].split(/\r?\n/)) {
    if (!line.trim() || /^\s*#/.test(line)) continue
    const topKey = /^([a-zA-Z0-9_-]+)\s*:\s*(.*)$/.exec(line)
    if (topKey) {
      const key = topKey[1]!
      const rest = topKey[2]!.trim()
      if (key === 'qmd') {
        currentListKey = null
        continue
      }
      if (!rest) {
        currentListKey = key
        if (!entries.has(key)) entries.set(key, [])
      } else {
        currentListKey = null
        if (rest.startsWith('[') && rest.endsWith(']')) {
          const items = rest
            .slice(1, -1)
            .split(',')
            .map(unquoteYaml)
            .filter(Boolean)
          if (items.length) entries.set(key, items)
        } else {
          const clean = unquoteYaml(rest)
          if (clean) entries.set(key, [clean])
        }
      }
      continue
    }
    if (currentListKey) {
      const listItem = /^\s+-\s+(.*)$/.exec(line)
      if (listItem) {
        const clean = unquoteYaml(listItem[1]!)
        if (clean) entries.get(currentListKey)?.push(clean)
        continue
      }
      currentListKey = null
    }
  }

  entries.delete('title')
  entries.delete('qmd')
  if (entries.has('sources') && (entries.get('sources')?.length ?? 0) > 0) {
    entries.delete('source_file')
    entries.delete('source_count')
  }

  const priority = ['category', 'status', 'tags', 'sources', 'source_file', 'updated_at', 'created_at', 'date', 'summary']
  const keys = [
    ...priority.filter(k => entries.has(k)),
    ...[...entries.keys()].filter(k => !priority.includes(k)),
  ]

  const result: MarkdownProperty[] = []
  for (const key of keys) {
    const values = (entries.get(key) ?? []).filter(Boolean)
    if (!values.length) continue
    if (key === 'category' || key === 'status') {
      result.push({ key, label: formatPropertyLabel(key), kind: 'badge', values })
    } else if (key === 'tags') {
      result.push({ key, label: formatPropertyLabel(key), kind: 'tags', values })
    } else if (key === 'sources' || key === 'source_file') {
      result.push({ key, label: formatPropertyLabel(key), kind: 'code-list', values })
    } else if (key === 'updated_at' || key === 'created_at' || key === 'date') {
      result.push({
        key,
        label: formatPropertyLabel(key),
        kind: 'date',
        values: values.map(formatPropertyDate),
        raw: values[0],
      })
    } else {
      result.push({ key, label: formatPropertyLabel(key), kind: 'text', values })
    }
  }
  return result
}

const escape = (value: string) => value.replace(/&/g, '&amp;').replace(/</g, '&lt;')
  .replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#39;')

const slug = (value: string) =>
  value.trim().toLocaleLowerCase().replace(/[^\p{L}\p{N}\s-]/gu, '').replace(/\s+/g, '-').replace(/-+/g, '-') || 'section'

export function markdownOutline(markdown: string): MarkdownHeading[] {
  const body = stripMarkdownFrontmatter(markdown)
  const used = new Map<string, number>()
  let fence: string | null = null
  const result: MarkdownHeading[] = []
  for (const line of body.split(/\r?\n/)) {
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

const IMAGE_EXT_RE = /\.(?:png|jpe?g|gif|webp|svg|avif|bmp|ico|tiff?)(?:[?#].*)?$/i
const VIDEO_FILE_EXT_RE = /\.(?:mp4|webm|mov|m4v|ogv|ogg|mkv|avi|m3u8)(?:[?#].*)?$/i
const VIDEO_PLATFORM_RE =
  /^https?:\/\/(?:(?:www\.|m\.|music\.)?youtube\.com\/(?:watch\?(?:.*&)?v=[^&\s]+|shorts\/[^/?#\s]+|embed\/[^/?#\s]+|live\/[^/?#\s]+)|(?:www\.)?youtu\.be\/[^/?#\s]+|player\.vimeo\.com\/video\/[^/?#\s]+|(?:www\.)?vimeo\.com\/\d+|(?:www\.)?dailymotion\.com\/video\/[^/?#\s]+|dai\.ly\/[^/?#\s]+|(?:www\.)?loom\.com\/(?:share|embed)\/[^/?#\s]+)/i

export function isMediaUrl(rawUrl: string): boolean {
  const clean = rawUrl.trim().replace(/^<|>$/g, '').split(/\s+/)[0] || ''
  return IMAGE_EXT_RE.test(clean) || VIDEO_FILE_EXT_RE.test(clean) || VIDEO_PLATFORM_RE.test(clean)
}

export function normalizeMediaMarkdownLinks(markdown: string): string {
  let fence: string | null = null
  return markdown
    .split(/\r?\n/)
    .map(line => {
      const delimiter = /^\s*(\x60{3,}|~{3,})/.exec(line)?.[1]
      if (delimiter) {
        if (fence === null) fence = delimiter[0]
        else if (delimiter[0] === fence) fence = null
        return line
      }
      if (fence !== null || !line.includes('](')) return line
      return line.replace(
        /(\x60+[^\x60]*\x60+)|(!?\[\[[^\]\r\n]+\]\])|(!?\[([^\]\r\n]*)\]\(([^)\r\n]+)\))/g,
        (full, codeSpan, wikiLink, mdLink, label, target) => {
          if (codeSpan || wikiLink) return full
          if (mdLink && !mdLink.startsWith('![') && isMediaUrl(target || '')) {
            return `![${label ?? ''}](${target})`
          }
          return full
        },
      )
    })
    .join('\n')
}

function renderMediaToken(alt: string, url: string): string {
  if (VIDEO_FILE_EXT_RE.test(url)) {
    const label = escape(alt || url)
    return '<video controls preload="metadata" src="' + escape(url) + '"><a href="' +
      escape(url) + '" rel="noopener noreferrer" target="_blank">' + label + '</a></video>'
  }
  if (VIDEO_PLATFORM_RE.test(url)) {
    const label = escape(alt || url)
    return '<a href="' + escape(url) + '" class="markdown-media-embed" data-media-embed="video" rel="noopener noreferrer" target="_blank">' +
      label + '</a>'
  }
  return '<img alt="' + escape(alt) + '" src="' + escape(url) + '" loading="lazy">'
}

function inline(value: string): string {
  const pattern = /(!?\[\[[^\]\r\n]+\]\]|!\[([^\]\r\n]*)\]\(([^)\r\n]+)\)|\[([^\]\r\n]+)\]\(([^)\r\n]+)\)|\x60[^\x60\r\n]+\x60|\*\*[^*\r\n]+\*\*|\*[^*\r\n]+\*|~~[^~\r\n]+~~)/g
  let output = ''
  let at = 0
  for (const match of value.matchAll(pattern)) {
    const start = match.index ?? 0
    output += escape(value.slice(at, start))
    const token = match[0]
    if (token.startsWith('[[') || token.startsWith('![[')) {
      const parts = token.slice(token.startsWith('!') ? 3 : 2, -2).split(/\\?\|/, 2)
      const target = parts[0].replace(/\\+$/, '').trim()
      output += target ? '<a href="#" class="knowledge-wikilink" data-wikilink="' +
        escape(target) + '">' + escape((parts[1] || target).trim()) + '</a>' : escape(token)
    } else if (token.startsWith('![')) {
      const url = safeUrl(match[3] || '')
      output += url ? renderMediaToken(match[2] || '', url) : escape(token)
    } else if (token.startsWith('[')) {
      const url = safeUrl(match[5] || '')
      if (url && isMediaUrl(url)) {
        output += renderMediaToken(match[4] || '', url)
      } else {
        output += url ? '<a href="' + escape(url) + '" rel="noopener noreferrer" target="_blank">' +
          escape(match[4] || '') + '</a>' : escape(token)
      }
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

type TableAlignment = 'left' | 'center' | 'right' | null

function splitTableRow(line: string): string[] {
  let s = line.trim()
  if (s.startsWith('|')) s = s.slice(1)
  if (s.endsWith('|') && !s.endsWith('\\|')) s = s.slice(0, -1)
  const cells: string[] = []
  let cell = ''
  let inCode = false
  let wikiDepth = 0
  for (let i = 0; i < s.length; i++) {
    const ch = s[i]!
    const next = s[i + 1]
    if (ch === '\x60') {
      inCode = !inCode
      cell += ch
      continue
    }
    if (!inCode && ch === '[' && next === '[') {
      wikiDepth++
      cell += '[['
      i++
      continue
    }
    if (!inCode && wikiDepth > 0 && ch === ']' && next === ']') {
      wikiDepth--
      cell += ']]'
      i++
      continue
    }
    if (!inCode && ch === '\\' && next === '|') {
      cell += wikiDepth > 0 ? '\\|' : '|'
      i++
      continue
    }
    if (!inCode && wikiDepth === 0 && ch === '|') {
      cells.push(cell.trim())
      cell = ''
      continue
    }
    cell += ch
  }
  cells.push(cell.trim())
  return cells
}

function parseTableAlignments(line: string): TableAlignment[] | null {
  const trimmed = line.trim()
  if (!trimmed.includes('|')) return null
  const cells = splitTableRow(trimmed)
  if (cells.length === 0 || !cells.every(cell => /^:?-{3,}:?$/.test(cell))) return null
  return cells.map(cell => {
    if (cell.startsWith(':') && cell.endsWith(':')) return 'center'
    if (cell.endsWith(':')) return 'right'
    if (cell.startsWith(':')) return 'left'
    return null
  })
}

function alignAttr(align: TableAlignment | undefined): string {
  return align ? ' style="text-align:' + align + '"' : ''
}

export function renderMarkdown(markdown: string): string {
  const body = stripMarkdownFrontmatter(markdown)
  const headings = markdownOutline(body)
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
  const lines = body.split(/\r?\n/)
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!
    const delimiter = /^\s*(\x60{3,}|~{3,})\s*([\w-]*)/.exec(line)
    if (code !== null) {
      if (delimiter && delimiter[1][0] === fence) {
        const text = escape(code.join('\n'))
        const lang = language.toLowerCase()
        output += lang === 'mermaid'
          ? '<pre class="knowledge-mermaid" data-mermaid-source="true">' + text + '</pre>'
          : lang === 'trading-chart'
            ? '<pre class="knowledge-trading-chart" data-trading-chart-source="true">' + text + '</pre>'
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
    if (line.includes('|') && i + 1 < lines.length) {
      const aligns = parseTableAlignments(lines[i + 1]!)
      if (aligns !== null) {
        const headers = splitTableRow(line)
        if (headers.length > 0) {
          flushParagraph(); closeList()
          const colCount = Math.max(headers.length, aligns.length)
          let tableHtml = '<div class="markdown-table-wrap"><table><thead><tr>'
          for (let c = 0; c < colCount; c++) {
            tableHtml += '<th' + alignAttr(aligns[c]) + '>' + inline(headers[c] ?? '') + '</th>'
          }
          tableHtml += '</tr></thead>'
          i += 2
          const bodyRows: string[] = []
          while (i < lines.length) {
            const rowLine = lines[i]!
            const rowTrim = rowLine.trim()
            if (
              !rowTrim ||
              !rowTrim.includes('|') ||
              /^\s*(\x60{3,}|~{3,})/.test(rowLine) ||
              /^(#{1,6})\s+/.test(rowLine) ||
              /^\s*(-{3,}|\*{3,}|_{3,})\s*$/.test(rowLine) ||
              /^\s*>/.test(rowLine)
            ) {
              break
            }
            const cells = splitTableRow(rowLine)
            let rowHtml = '<tr>'
            for (let c = 0; c < colCount; c++) {
              rowHtml += '<td' + alignAttr(aligns[c]) + '>' + inline(cells[c] ?? '') + '</td>'
            }
            rowHtml += '</tr>'
            bodyRows.push(rowHtml)
            i++
          }
          if (bodyRows.length) {
            tableHtml += '<tbody>' + bodyRows.join('') + '</tbody>'
          }
          tableHtml += '</table></div>'
          output += tableHtml
          i--
          continue
        }
      }
    }
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

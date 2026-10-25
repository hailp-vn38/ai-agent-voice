/** Read-only, escaped Markdown output for the Knowledge reading view. */
export type Heading = { id: string; level: number; title: string }

function escapeHtml(value: string): string {
  return value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;').replace(/'/g, '&#39;')
}

function slug(title: string): string {
  return title.toLocaleLowerCase().replace(/[^\p{L}\p{N}\s-]/gu, '').trim()
    .replace(/\s+/g, '-').replace(/-+/g, '-') || 'section'
}

export function getHeadings(source: string): Heading[] {
  const counts = new Map<string, number>()
  let fence = ''
  const results: Heading[] = []
  for (const line of source.split(/\r?\n/)) {
    const marker = /^\s*(\x60{3,}|~{3,})/.exec(line)
    if (marker) {
      if (!fence) fence = marker[1][0]
      else if (marker[1][0] === fence) fence = ''
      continue
    }
    if (fence) continue
    const heading = /^(#{1,6})\s+(.+?)\s*#*\s*$/.exec(line)
    if (!heading) continue
    const title = heading[2].replace(/[*_\x60]/g, '').trim()
    const id = slug(title)
    const n = counts.get(id) ?? 0
    counts.set(id, n + 1)
    results.push({ id: n ? id + '-' + n : id, level: heading[1].length, title })
  }
  return results
}

export function getWikiTargets(source: string): string[] {
  const result: string[] = []
  const seen = new Set<string>()
  for (const match of source.matchAll(/!?\[\[([^\]\r\n]+)\]\]/g)) {
    const target = match[1].split('|')[0]?.split('#')[0]?.trim() ?? ''
    if (target && !seen.has(target)) { result.push(target); seen.add(target) }
  }
  return result
}

function safeHref(href: string): string | null {
  if (/^(https?:\/\/|mailto:|\/(?!\/)|\.{1,2}\/|#)/i.test(href.trim())) return href.trim()
  return null
}

const IMAGE_EXT_RE = /\.(?:png|jpe?g|gif|webp|svg|avif|bmp|ico|tiff?)(?:[?#].*)?$/i
const VIDEO_FILE_EXT_RE = /\.(?:mp4|webm|mov|m4v|ogv|ogg|mkv|avi|m3u8)(?:[?#].*)?$/i
const VIDEO_PLATFORM_RE =
  /^https?:\/\/(?:(?:www\.|m\.|music\.)?youtube\.com\/(?:watch\?(?:.*&)?v=[^&\s]+|shorts\/[^/?#\s]+|embed\/[^/?#\s]+|live\/[^/?#\s]+)|(?:www\.)?youtu\.be\/[^/?#\s]+|player\.vimeo\.com\/video\/[^/?#\s]+|(?:www\.)?vimeo\.com\/\d+|(?:www\.)?dailymotion\.com\/video\/[^/?#\s]+|dai\.ly\/[^/?#\s]+|(?:www\.)?loom\.com\/(?:share|embed)\/[^/?#\s]+)/i

function isMediaHref(rawUrl: string): boolean {
  const clean = rawUrl.trim().replace(/^<|>$/g, '').split(/\s+/)[0] || ''
  return IMAGE_EXT_RE.test(clean) || VIDEO_FILE_EXT_RE.test(clean) || VIDEO_PLATFORM_RE.test(clean)
}

function renderMediaHref(alt: string, href: string): string {
  if (VIDEO_FILE_EXT_RE.test(href)) {
    const label = escapeHtml(alt || href)
    return '<video controls preload="metadata" src="' + escapeHtml(href) + '"><a href="' +
      escapeHtml(href) + '" target="_blank" rel="noopener noreferrer">' + label + '</a></video>'
  }
  if (VIDEO_PLATFORM_RE.test(href)) {
    const label = escapeHtml(alt || href)
    return '<a href="' + escapeHtml(href) + '" class="markdown-media-embed" data-media-embed="video" target="_blank" rel="noopener noreferrer">' +
      label + '</a>'
  }
  return '<img alt="' + escapeHtml(alt) + '" src="' + escapeHtml(href) + '" loading="lazy">'
}

function inlineMarkdown(source: string): string {
  const token = /(!?\[\[[^\]\r\n]+\]\]|!\[([^\]\r\n]*)\]\(([^)\r\n]+)\)|\[([^\]\r\n]+)\]\(([^)\r\n]+)\)|\x60[^\x60\r\n]+\x60|\*\*[^*\r\n]+\*\*|~~[^~\r\n]+~~|\*[^*\r\n]+\*)/g
  let result = ''
  let from = 0
  for (const match of source.matchAll(token)) {
    const start = match.index ?? 0
    result += escapeHtml(source.slice(from, start))
    const raw = match[0]
    if (raw.startsWith('[[') || raw.startsWith('![[')) {
      const contents = raw.slice(raw.startsWith('!') ? 3 : 2, -2)
      const [target, label] = contents.split('|', 2)
      const cleanTarget = target?.trim() ?? ''
      result += cleanTarget
        ? '<a href="#" class="wikilink" data-wikilink="' + escapeHtml(cleanTarget) +
          '">' + escapeHtml((label || cleanTarget).trim()) + '</a>'
        : escapeHtml(raw)
    } else if (raw.startsWith('![')) {
      const href = safeHref(match[3] || '')
      result += href ? renderMediaHref(match[2] || '', href) : escapeHtml(raw)
    } else if (raw.startsWith('[')) {
      const href = safeHref(match[5] || '')
      if (href && isMediaHref(href)) {
        result += renderMediaHref(match[4] || '', href)
      } else {
        result += href
          ? '<a href="' + escapeHtml(href) + '" target="_blank" rel="noopener noreferrer">' +
            escapeHtml(match[4] || '') + '</a>'
          : escapeHtml(raw)
      }
    } else if (raw.startsWith('\x60')) {
      result += '<code>' + escapeHtml(raw.slice(1, -1)) + '</code>'
    } else if (raw.startsWith('**')) {
      result += '<strong>' + escapeHtml(raw.slice(2, -2)) + '</strong>'
    } else if (raw.startsWith('~~')) {
      result += '<del>' + escapeHtml(raw.slice(2, -2)) + '</del>'
    } else {
      result += '<em>' + escapeHtml(raw.slice(1, -1)) + '</em>'
    }
    from = start + raw.length
  }
  return result + escapeHtml(source.slice(from))
}

export function renderMarkdown(source: string, skipLeadingTitle = ''): string {
  const lines = source.split(/\r?\n/)
  const headings = getHeadings(source)
  let headingPosition = 0
  let html = ''
  let paragraph: string[] = []
  let list: 'ul' | 'ol' | null = null
  let code: string[] | null = null
  let fence = ''
  const flush = () => {
    if (paragraph.length) html += '<p>' + inlineMarkdown(paragraph.join(' ')) + '</p>'
    paragraph = []
  }
  const closeList = () => { if (list) html += '</' + list + '>'; list = null }
  for (const line of lines) {
    const matchFence = /^\s*(\x60{3,}|~{3,})\s*([\w-]*)/.exec(line)
    if (code !== null) {
      if (matchFence && matchFence[1][0] === fence) {
        html += '<pre><code>' + escapeHtml(code.join('\n')) + '</code></pre>'
        code = null
        fence = ''
      } else code.push(line)
      continue
    }
    if (matchFence) { flush(); closeList(); fence = matchFence[1][0]; code = []; continue }
    if (!line.trim()) { flush(); closeList(); continue }
    const heading = /^(#{1,6})\s+(.+?)\s*#*\s*$/.exec(line)
    if (heading) {
      flush(); closeList()
      const data = headings[headingPosition++]
      if (data.level === 1 && headingPosition === 1 &&
        data.title.toLocaleLowerCase() === skipLeadingTitle.toLocaleLowerCase()) continue
      const level = heading[1].length
      html += '<h' + level + ' id="' + escapeHtml(data.id) + '">' +
        inlineMarkdown(heading[2]) + '</h' + level + '>'
      continue
    }
    if (/^\s*(-{3,}|\*{3,}|_{3,})\s*$/.test(line)) {
      flush(); closeList(); html += '<hr>'; continue
    }
    const quote = /^\s*>\s?(.*)$/.exec(line)
    if (quote) { flush(); closeList(); html += '<blockquote><p>' + inlineMarkdown(quote[1]) + '</p></blockquote>'; continue }
    const unordered = /^\s*[-*+]\s+(.*)$/.exec(line)
    const ordered = /^\s*\d+\.\s+(.*)$/.exec(line)
    if (unordered || ordered) {
      flush()
      const kind = unordered ? 'ul' : 'ol'
      if (kind !== list) { closeList(); list = kind; html += '<' + kind + '>' }
      let content = (unordered || ordered)![1]
      const task = /^\[([ xX])\]\s+/.exec(content)
      if (task) {
        content = content.slice(task[0].length)
        html += '<li>' + (task[1].toLowerCase() === 'x' ? '☑ ' : '☐ ') + inlineMarkdown(content) + '</li>'
      } else html += '<li>' + inlineMarkdown(content) + '</li>'
      continue
    }
    closeList()
    paragraph.push(line.trim())
  }
  flush(); closeList()
  if (code !== null) html += '<pre><code>' + escapeHtml(code.join('\n')) + '</code></pre>'
  return html
}

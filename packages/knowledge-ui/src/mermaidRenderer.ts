/**
 * Render Markdown Mermaid fences on demand. The pinned ESM release is loaded only
 * when a diagram is visible; normal Markdown remains fully local and escaped.
 * On CDN/offline failure the source fence remains legible.
 */
type Mermaid = {
  initialize(config: Record<string, unknown>): void
  render(id: string, source: string): Promise<{ svg: string }>
}
const MERMAID_URL = 'https://cdn.jsdelivr.net/npm/mermaid@11.17.2/dist/mermaid.esm.min.mjs'
let loader: Promise<Mermaid> | null = null
let nextId = 0

// Mermaid's `strict` level still loads images, styles, and config-supplied CSS while it measures.
const REMOTE_CONTENT = /<\s*(img|image|svg|video|audio|source|iframe|object|embed|link|style)\b|url\s*\(|image-set|@import|\\[0-9a-f]|%%\{|@\{[^}]*\bimg\b/i

/** True when a diagram could make the browser contact another host or change its config. */
export function canLoadRemoteContent(source: string): boolean {
  return source.trimStart().startsWith('---') || REMOTE_CONTENT.test(source)
}

async function loadMermaid(): Promise<Mermaid> {
  if (!loader) {
    loader = import(/* @vite-ignore */ MERMAID_URL)
      .then((module: { default: Mermaid }) => module.default)
      .catch((cause: unknown) => { loader = null; throw cause })
  }
  return loader
}

/**
 * Set `untrustedSource` for text no person wrote, such as a model's answer. A diagram
 * that could load remote content then stays as source and never reaches Mermaid.
 */
export async function renderMermaidBlocks(root: HTMLElement | null, dark: boolean, untrustedSource = false): Promise<void> {
  if (!root) return
  const blocks = Array.from(root.querySelectorAll<HTMLElement>('pre.knowledge-mermaid[data-mermaid-source]'))
  if (!blocks.length) return
  let mermaid: Mermaid
  try {
    mermaid = await loadMermaid()
    mermaid.initialize({
      startOnLoad: false,
      securityLevel: 'strict',
      suppressErrorRendering: true,
      theme: dark ? 'dark' : 'default',
      maxTextSize: 50000,
    })
  } catch {
    // Preserve escaped source when the pinned CDN cannot be reached.
    return
  }

  for (const block of blocks) {
    if (!block.isConnected || block.dataset.mermaidSource !== 'true') continue
    const source = block.textContent || ''
    if (untrustedSource && canLoadRemoteContent(source)) {
      block.dataset.mermaidSource = 'blocked'
      continue
    }
    // An overlapping pass must not render the same block again.
    block.dataset.mermaidSource = 'rendering'
    try {
      const { svg } = await mermaid.render('knowledge-mermaid-' + ++nextId, source)
      if (!block.isConnected || block.textContent !== source) continue
      const container = document.createElement('div')
      container.className = 'knowledge-mermaid-diagram'
      container.setAttribute('role', 'img')
      container.setAttribute('aria-label', 'Mermaid diagram')
      container.innerHTML = svg
      block.replaceWith(container)
    } catch {
      // Syntax errors must not hide the original Markdown source.
      block.dataset.mermaidSource = 'error'
    }
  }
}

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

async function loadMermaid(): Promise<Mermaid> {
  if (!loader) {
    loader = import(/* @vite-ignore */ MERMAID_URL)
      .then((module: { default: Mermaid }) => module.default)
      .catch((cause: unknown) => { loader = null; throw cause })
  }
  return loader
}

export async function renderMermaidBlocks(root: HTMLElement | null, dark: boolean): Promise<void> {
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

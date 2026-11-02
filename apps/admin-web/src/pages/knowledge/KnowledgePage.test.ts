import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, it, vi } from 'vitest'
import { setLocale } from '@/composables/useI18n'
import KnowledgePage from './KnowledgePage.vue'
import WikiChatPanel from './WikiChatPanel.vue'
import KnowledgeGraph from './KnowledgeGraph.vue'

const { requestJsonMock, routeQuery, focusNodeMock, highlightNodeMock } = vi.hoisted(() => ({
  requestJsonMock: vi.fn(), routeQuery: {} as Record<string,string>,
  focusNodeMock: vi.fn(), highlightNodeMock: vi.fn(),
}))
vi.mock('@/api/client', () => ({
  apiUrl: (path: string) => path,
  requestJson: requestJsonMock,
  jsonRequest: (method: string, body: unknown) => ({ method, body }),
  withQuery: (path: string, query: Record<string, string | number>) =>
    path + '?' + new URLSearchParams(
      Object.entries(query).map(([key, value]) => [key, String(value)]),
    ).toString(),
}))
vi.mock('vue-router', () => ({ useRoute: () => ({ query: routeQuery }) }))
vi.mock('./KnowledgeGraph.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({
    props: { centerId: String, reloadToken: Number, focusMode: Boolean, answerGraph: Object, inactive: Boolean },
    emits: ['node-hover'],
    setup(props, { expose, emit }) {
      expose({ focusNode: focusNodeMock, highlightNode: highlightNodeMock })
      return () => h('div', { class: 'mock-knowledge-graph', 'data-revision': props.answerGraph?.revision }, [
        h('span', 'Knowledge graph'),
        h('button', { class: 'mock-node-hover', onMouseenter: () => emit('node-hover', 'wiki/concepts/known.md') }, 'Hover node'),
      ])
    },
  }) }
})

beforeEach(() => {
  vi.clearAllMocks()
  for (const key of Object.keys(routeQuery)) delete routeQuery[key]
  window.localStorage.clear()
  setLocale('en')
  requestJsonMock.mockImplementation(async (url: string) => {
    if (url.startsWith('/api/admin/knowledge/tree?')) {
      const space = new URL(url, 'http://localhost').searchParams.get('space')
      return { items: [
        { type: 'folder', space, path: space, name: space, editable: space !== 'drafts' },
        { type: 'file', space, path: space + '/' + space + '-file.md',
          name: space + '-file.md', editable: space !== 'drafts',
          ...(space !== 'raw' ? { id: 'wiki_demo', revision: 1, status: 'verified' } : {}) },
      ] }
    }
    if (url === '/api/admin/knowledge/index') return {
      health: 'ready', document_count: 1, last_complete_scan_at: null,
    }
    if (url === '/api/admin/knowledge/sources') return { items: [] }
    if (url === '/api/admin/knowledge/pages/wiki_demo') return {
      id: 'wiki_demo', title: 'Wiki Demo',
      body: '# Wiki Demo\n\nA linked, readable document.\n\n## Overview\n\n**Bold** [[wiki-file|Related Wiki]] and <script>alert(1)</script>',
      status: 'verified', revision: 1, source_ref: null,
    }
    if (url.startsWith('/api/admin/knowledge/pages')) return { items: [], next_cursor: null }
    if (url.startsWith('/api/admin/knowledge/graph')) return { nodes: [], edges: [] }
    if (url.startsWith('/api/admin/knowledge/raw/files?')) return {
      path: 'raw/raw-file.md', body: '# Raw\n\nRaw content', sha256: 'hash',
    }
    throw new Error('Unexpected request ' + url)
  })
})

it('matches option A with a read-only Markdown document, tabs, explorer and outline', async () => {
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    expect(wrapper.find('.knowledge-a').exists()).toBe(true)
    expect(wrapper.find('.a-ribbon').exists()).toBe(false)
    expect(wrapper.find('[aria-label="Vault tools"]').exists()).toBe(false)
    expect(wrapper.find('.a-explorer').exists()).toBe(true)
    expect(wrapper.find('.a-outline').text()).toContain('On this page')
    expect(wrapper.find('.a-outline').text()).toContain('Linked notes')
    expect(wrapper.find('.doc h1').text()).toBe('Wiki Demo')
    expect(wrapper.find('.markdown-body strong').text()).toBe('Bold')
    expect(wrapper.find('.outline-link').text()).toBe('Overview')
    expect(wrapper.find('.markdown-body script').exists()).toBe(false)
    expect(wrapper.find('.markdown-body').text()).toContain('<script>alert(1)</script>')
    expect(wrapper.find('input').exists()).toBe(false)
    expect(wrapper.find('textarea').exists()).toBe(false)
  } finally { wrapper.unmount() }
})

it('offers Markdown download, link and content copying in the note menu with an icon-only reading button', async () => {
  const writeText = vi.fn().mockResolvedValue(undefined)
  const createObjectURL = vi.fn((_blob: Blob) => 'blob:note-download')
  const revokeObjectURL = vi.fn()
  vi.stubGlobal('navigator', Object.create(navigator, { clipboard: { value: { writeText } } }))
  vi.stubGlobal('URL', class extends URL {
    static createObjectURL = createObjectURL
    static revokeObjectURL = revokeObjectURL
  })
  let filename = ''
  const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
    filename = this.download
  })
  const wrapper = mount(KnowledgePage, { attachTo: document.body })
  try {
    await flushPromises()
    const reading = wrapper.get('button[aria-label="Reading view"]')
    expect(reading.text()).toBe('')
    expect(reading.find('svg').exists()).toBe(true)
    const menu = wrapper.get('button[aria-label="Note actions"]')
    await menu.trigger('click')
    await flushPromises()
    const items = Array.from(document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'))
    expect(items.map(item => item.textContent?.trim())).toEqual(['Download', 'Copy note link', 'Copy Markdown content'])
    items[0]!.click()
    await flushPromises()
    expect(filename).toBe('wiki-file.md')
    const blob = createObjectURL.mock.calls[0]![0]
    const contents = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader()
      reader.onload = () => resolve(String(reader.result))
      reader.onerror = () => reject(reader.error)
      reader.readAsText(blob)
    })
    expect(contents).toBe((await requestJsonMock('/api/admin/knowledge/pages/wiki_demo')).body)
    await menu.trigger('click')
    await flushPromises()
    document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')[1]!.click()
    await flushPromises()
    expect(writeText).toHaveBeenCalledWith(window.location.origin + '/knowledge#wiki_demo')
    await menu.trigger('click')
    await flushPromises()
    document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')[2]!.click()
    await flushPromises()
    expect(writeText).toHaveBeenLastCalledWith(contents)
    expect(wrapper.find('.a-notice').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('Note link copied')
  } finally {
    wrapper.unmount()
    click.mockRestore()
    vi.unstubAllGlobals()
  }
})

it('defaults to Knowledge and switches its single displayed tree using the source tabs', async () => {
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    expect(wrapper.find('select[aria-label="Knowledge source"]').exists()).toBe(false)
    expect(wrapper.get('.a-pane-heading').text()).not.toContain('File explorer')
    const tablist = wrapper.get('.a-pane-heading [role="tablist"][aria-label="Knowledge source"]')
    const tabs = tablist.findAll('[role="tab"]')
    expect(tabs.map(tab => tab.text())).toEqual(['', ''])
    expect(tabs.map(tab => tab.attributes('aria-label'))).toEqual(['Knowledge', 'File'])
    expect(tabs.map(tab => tab.attributes('title'))).toEqual(['Knowledge', 'File'])
    expect(tabs.every(tab => tab.find('svg').exists())).toBe(true)
    expect(tabs[0]!.attributes('aria-selected')).toBe('true')
    expect(tabs[1]!.attributes('aria-selected')).toBe('false')
    expect(wrapper.text()).toContain('Wiki Demo')
    expect(wrapper.text()).not.toContain('raw-file')

    await tabs[1]!.trigger('click')
    await flushPromises()
    expect(tabs[0]!.attributes('aria-selected')).toBe('false')
    expect(tabs[1]!.attributes('aria-selected')).toBe('true')
    expect(requestJsonMock).toHaveBeenCalledWith('/api/admin/knowledge/tree?space=raw')
    expect(wrapper.find('.tree-holder').text()).toContain('raw-file')
    expect(wrapper.find('.tree-holder').text()).not.toContain('wiki-file')

    await tabs[0]!.trigger('click')
    await flushPromises()
    expect(tabs[0]!.attributes('aria-selected')).toBe('true')
    expect(tabs[1]!.attributes('aria-selected')).toBe('false')
    expect(wrapper.find('.tree-holder').text()).toContain('Wiki Demo')
    expect(wrapper.find('.tree-holder').text()).not.toContain('raw-file')
  } finally { wrapper.unmount() }
})

it('scrolls an overflowing tab list sideways with the mouse wheel', async () => {
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    const list = wrapper.get('.a-tab-list')
    const element = list.element as HTMLElement

    await list.trigger('wheel', { deltaY: 120 })
    expect(element.scrollLeft).toBe(0)

    Object.defineProperty(element, 'scrollWidth', { configurable: true, value: 900 })
    Object.defineProperty(element, 'clientWidth', { configurable: true, value: 300 })
    await list.trigger('wheel', { deltaY: 120 })
    expect(element.scrollLeft).toBe(120)
  } finally { wrapper.unmount() }
})

it('supports rendered Wiki links and the existing Graph tab without editor forms', async () => {
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    const link = wrapper.find('a[data-wikilink]')
    expect(link.text()).toBe('Related Wiki')
    await link.trigger('click')
    await flushPromises()
    expect(requestJsonMock).toHaveBeenCalledWith('/api/admin/knowledge/pages/wiki_demo')
    await wrapper.find('button.other-tab').trigger('click')
    expect(wrapper.text()).toContain('Knowledge graph')
    expect(wrapper.find('.wiki-chat-panel textarea').exists()).toBe(true)
    expect(wrapper.get('.knowledge-a').classes()).toContain('is-graph-view')
  } finally { wrapper.unmount() }
})

it('streams a Wiki answer, switches to its answer graph, and keeps chat when changing tabs', async () => {
  const graph = {
    revision: 1,
    nodes: [{
      id: 'wiki/concepts/known.md', path: 'wiki/concepts/known.md', title: 'Known',
      status: 'verified', revision: 1, tier: 'retrieved', score: 0.8,
    }],
    edges: [], unresolved: [], truncated: false,
  }
  const sse = [
    'event: tool\ndata: {"call_id":"c1","name":"wiki_query","state":"completed"}\n\n',
    `event: graph\ndata: ${JSON.stringify(graph)}\n\n`,
    'event: delta\ndata: {"text":"A **known** page."}\n\n',
    'event: done\ndata: {"citations":["wiki/concepts/known.md"],"stop_reason":"finished"}\n\n',
  ].join('')
  const fetchMock = vi.fn().mockResolvedValue(new Response(sse, {
    headers: { 'Content-Type': 'text/event-stream' },
  }))
  vi.stubGlobal('fetch', fetchMock)
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    await wrapper.get('button.other-tab').trigger('click')
    await flushPromises()
    expect(wrapper.find('.wiki-chat-panel').exists()).toBe(true)
    await wrapper.get('.wiki-chat-panel textarea').setValue('What do we know?')
    expect(wrapper.get('.wiki-chat-send').attributes('disabled')).toBeUndefined()
    await wrapper.get('.wiki-chat-panel form').trigger('submit')
    expect(wrapper.findComponent(WikiChatPanel).emitted('send')).toEqual([['What do we know?']])
    await flushPromises()

    expect(fetchMock).toHaveBeenCalledOnce()
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(url).toContain('/api/admin/knowledge/ask')
    expect(init.method).toBe('POST')
    expect(JSON.parse(String(init.body))).toEqual({
      messages: [{ role: 'user', content: 'What do we know?' }],
    })
    expect(wrapper.get('.wiki-chat-answer strong').text()).toBe('known')
    expect(wrapper.get('.mock-knowledge-graph').attributes('data-revision')).toBe('1')

    await wrapper.get('button.a-tab').trigger('click')
    await flushPromises()
    await wrapper.get('button.other-tab').trigger('click')
    await flushPromises()
    expect(wrapper.get('.wiki-chat-answer').text()).toContain('A known page.')
    expect(wrapper.get('.mock-knowledge-graph').attributes('data-revision')).toBe('1')
  } finally {
    wrapper.unmount()
    vi.unstubAllGlobals()
  }
})

it('attaches the open note to the chat, naming a File by its Source path', async () => {
  const sse = 'event: done\ndata: {"citations":["sources/raw-file.md"],"stop_reason":"finished"}\n\n'
  const fetchMock = vi.fn(() => Promise.resolve(new Response(sse, { headers: { 'Content-Type': 'text/event-stream' } })))
  vi.stubGlobal('fetch', fetchMock)
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    await wrapper.findAll('.a-right-tab')[1]!.trigger('click')
    await flushPromises()
    await wrapper.get('.wiki-chat-file-add').trigger('click')
    expect(wrapper.get('.wiki-chat-files .wiki-chat-file').text()).toBe('Wiki Demo')

    const spaces = wrapper.findAll('.a-pane-heading [role="tab"]')
    await spaces[1]!.trigger('click')
    await flushPromises()
    await wrapper.findAll('button.tree-leaf').find(btn => btn.text().includes('raw-file'))!.trigger('click')
    await flushPromises()
    await wrapper.get('.wiki-chat-file-add').trigger('click')

    await wrapper.get('.wiki-chat-panel textarea').setValue('Compare these')
    await wrapper.get('.wiki-chat-panel form').trigger('submit')
    await flushPromises()
    const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit]
    expect(JSON.parse(String(init.body)).pinned_paths).toEqual(['wiki/wiki-file.md', 'sources/raw-file.md'])
  } finally {
    wrapper.unmount()
    vi.unstubAllGlobals()
  }
})

it('stops a stream and retains the partial assistant answer', async () => {
  let signal: AbortSignal | undefined
  const fetchMock = vi.fn((_url: string, init: RequestInit) => {
    signal = init.signal as AbortSignal
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode('event: delta\ndata: {"text":"Partial"}\n\n'))
      },
    })
    return Promise.resolve(new Response(stream, {
      headers: { 'Content-Type': 'text/event-stream' },
    }))
  })
  vi.stubGlobal('fetch', fetchMock)
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    await wrapper.get('button.other-tab').trigger('click')
    await flushPromises()
    await wrapper.get('.wiki-chat-panel textarea').setValue('Ask')
    await wrapper.get('.wiki-chat-panel form').trigger('submit')
    await flushPromises()
    expect(wrapper.get('.wiki-chat-answer').text()).toContain('Partial')
    await wrapper.get('.wiki-chat-stop').trigger('click')
    await flushPromises()
    expect(signal?.aborted).toBe(true)
    expect(wrapper.get('.wiki-chat-answer').text()).toContain('Partial')
    expect(wrapper.get('.wiki-chat-message.is-assistant').text()).toContain('Stopped')
  } finally {
    wrapper.unmount()
    vi.unstubAllGlobals()
  }
})

it('links only evidenced citations, highlights graph sources, restores an old answer graph, and summarizes tool pages', async () => {
  const original = requestJsonMock.getMockImplementation()!
  requestJsonMock.mockImplementation(async (url: string, options?: {method?: string; body?: Record<string, unknown>}) => {
    if (url === '/api/admin/knowledge/tree?space=wiki') return { items: [
      { type: 'folder', space: 'wiki', path: 'wiki', name: 'wiki', editable: true },
      { type: 'folder', space: 'wiki', path: 'wiki/concepts', name: 'concepts', editable: true },
      { type: 'folder', space: 'wiki', path: 'wiki/entities', name: 'entities', editable: true },
      { type: 'folder', space: 'wiki', path: 'wiki/syntheses', name: 'syntheses', editable: true },
    ] }
    if (url === '/api/admin/knowledge/pages/wiki%2Fconcepts%2Fknown.md') return {
      id: 'wiki/concepts/known.md', title: 'Known', body: 'Known body', status: 'verified', revision: 1, source_ref: null,
    }
    return original(url, options)
  })
  const graph = (revision: number) => ({ revision, nodes: [
    { id: 'wiki/concepts/known.md', path: 'wiki/concepts/known.md', title: 'Known', status: 'verified', revision: 1, tier: 'cited' },
    { id: 'wiki/concepts/second.md', path: 'wiki/concepts/second.md', title: 'Second source', status: 'verified', revision: 1, tier: 'cited' },
  ], edges: [], unresolved: [], truncated: false })
  const response = (revision: number) => new Response([
    `event: tool\ndata: ${JSON.stringify({ call_id: `c${revision}`, name: 'wiki_multi_get', state: 'completed', query: 'concepts/*.md', pages: ['wiki/concepts/known.md', 'wiki/concepts/second.md'] })}\n\n`,
    `event: graph\ndata: ${JSON.stringify(graph(revision))}\n\n`,
    'event: delta\ndata: {"text":"Sources: [[wiki/concepts/known.md|Known]], [[wiki/concepts/second.md|Second]], and [[wiki/concepts/invented.md|Made up]]."}\n\n',
    'event: done\ndata: {"citations":["wiki/concepts/known.md","wiki/concepts/second.md"],"stop_reason":"finished"}\n\n',
  ].join(''), { headers: { 'Content-Type': 'text/event-stream' } })
  const fetchMock = vi.fn().mockResolvedValueOnce(response(1)).mockResolvedValueOnce(response(2))
  vi.stubGlobal('fetch', fetchMock)
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    await wrapper.get('button.other-tab').trigger('click')
    await flushPromises()
    const panel = wrapper.get('.wiki-chat-panel')
    const suggestions = panel.findAll('.wiki-chat-suggestion')
    expect(suggestions).toHaveLength(3)
    expect(suggestions.map(button => button.text())).toEqual(expect.arrayContaining(['concepts', 'entities', 'syntheses']))
    await suggestions[0]!.trigger('click')
    await flushPromises()
    expect(wrapper.findAll('.wiki-chat-citation').map(link => link.text())).toEqual(['1', '2'])
    expect(wrapper.get('.wiki-chat-answer').text()).toContain('Made up')
    expect(wrapper.find('.wiki-chat-answer a[data-wikilink*="invented"]').exists()).toBe(false)
    expect(wrapper.findAll('.wiki-chat-sources li')).toHaveLength(2)
    const tool = wrapper.get('.wiki-chat-tool-summary')
    await tool.get('summary').trigger('click')
    expect(tool.text()).toContain('wiki/concepts/known.md')
    expect(tool.text()).toContain('concepts/*.md')

    const citation = wrapper.get('.wiki-chat-citation[data-citation-path="wiki/concepts/known.md"]')
    await citation.trigger('mouseover')
    expect(highlightNodeMock).toHaveBeenCalledWith('wiki/concepts/known.md')
    await citation.trigger('click')
    expect(focusNodeMock).toHaveBeenCalledWith('wiki/concepts/known.md', false)
    await flushPromises()
    expect(wrapper.get('.doc h1').text()).toBe('Known')
    expect((wrapper.get('.wiki-chat-panel').element as HTMLElement).style.display).not.toBe('none')
    expect(wrapper.get('.wiki-chat-answer').text()).toContain('Made up')
    await wrapper.get('.mock-node-hover').trigger('mouseenter')
    expect(wrapper.find('.wiki-chat-sources li.is-highlighted').exists()).toBe(true)

    await panel.get('textarea').setValue('Second question')
    await panel.get('form').trigger('submit')
    await flushPromises()
    expect(wrapper.get('.mock-knowledge-graph').attributes('data-revision')).toBe('2')
    const oldGraph = wrapper.findAll('.wiki-chat-show-graph')[0]
    await oldGraph!.trigger('click')
    await flushPromises()
    expect(wrapper.get('.mock-knowledge-graph').attributes('data-revision')).toBe('1')
  } finally {
    wrapper.unmount()
    vi.unstubAllGlobals()
  }
})

it('hides file and folder creation actions in the Knowledge tab and shows them only in the File tab', async () => {
  const original = requestJsonMock.getMockImplementation()!
  requestJsonMock.mockImplementation(async (url: string, options?: {method?: string; body?: Record<string, unknown>}) => {
    if (url === '/api/admin/knowledge/tree?space=wiki') return {
      items: [
        { type: 'folder', space: 'wiki', path: 'wiki', name: 'wiki', editable: true },
        { type: 'folder', space: 'wiki', path: 'wiki/AI', name: 'AI', editable: true },
      ],
    }
    if (url === '/api/admin/knowledge/tree?space=raw') return {
      items: [
        { type: 'folder', space: 'raw', path: 'raw', name: 'raw', editable: true },
        { type: 'folder', space: 'raw', path: 'raw/notes', name: 'notes', editable: true },
      ],
    }
    return original(url, options)
  })
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    expect(wrapper.find('button[aria-label="Add folder"]').exists()).toBe(false)
    expect(wrapper.find('button[aria-label="Add file"]').exists()).toBe(false)
    expect(wrapper.find('button[aria-label^="Create note in"]').exists()).toBe(false)

    const fileTab = wrapper.findAll('[role="tablist"][aria-label="Knowledge source"] [role="tab"]')
      .find(tab => tab.attributes('aria-label') === 'File')!
    await fileTab.trigger('click')
    await flushPromises()
    expect(wrapper.find('button[aria-label="Add folder"]').exists()).toBe(true)
    expect(wrapper.find('button[aria-label="Add file"]').exists()).toBe(true)
    expect(wrapper.find('button[aria-label="Create note in notes"]').exists()).toBe(true)
  } finally { wrapper.unmount() }
})

it('does not render its own sidebar controls inside the Knowledge page', async () => {
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    expect(wrapper.find('.a-ribbon').exists()).toBe(false)
    expect(wrapper.find('button[aria-label="Expand web sidebar"]').exists()).toBe(false)
    expect(wrapper.find('.a-explorer').exists()).toBe(true)
    expect(wrapper.find('.a-outline').exists()).toBe(true)
  } finally { wrapper.unmount() }
})


it('persists the selected vault and restores it on reload', async () => {
  const first = mount(KnowledgePage)
  await flushPromises()
  const fileTab = first.findAll('[role="tablist"][aria-label="Knowledge source"] [role="tab"]')
    .find(tab => tab.attributes('aria-label') === 'File')!
  await fileTab.trigger('click')
  await flushPromises()
  expect(window.localStorage.getItem('voice-agent-knowledge-vault-space')).toBe('raw')
  first.unmount()
  requestJsonMock.mockClear()
  const again = mount(KnowledgePage)
  try {
    await flushPromises()
    const restoredFileTab = again.findAll('[role="tablist"][aria-label="Knowledge source"] [role="tab"]')
      .find(tab => tab.attributes('aria-label') === 'File')!
    expect(restoredFileTab.attributes('aria-selected')).toBe('true')
    expect(requestJsonMock).toHaveBeenCalledWith('/api/admin/knowledge/tree?space=raw')
    expect(again.get('.tree-holder').text()).toContain('raw-file')
    expect(again.get('.tree-holder').text()).not.toContain('wiki-file')
  } finally { again.unmount() }
})

it('creates an empty Raw note in the selected folder and saves its content with CAS', async () => {
  const original = requestJsonMock.getMockImplementation()!
  const writes: Array<{url:string; options?:{method?: string;body?:Record<string,unknown>}}> = []
  requestJsonMock.mockImplementation(async (url: string, options?:{method?:string;body?:Record<string,unknown>}) => {
    if (url === '/api/admin/knowledge/tree?space=raw') return {
      items: [
        {type:'folder',space:'raw',path:'raw',name:'raw',editable:true},
        {type:'folder',space:'raw',path:'raw/notes',name:'notes',editable:true},
      ],
    }
    if (url === '/api/admin/knowledge/raw/files' && options?.method === 'POST') {
      writes.push({url, options})
      return {path:'raw/notes/Meeting.md',body:'',sha256:'empty-sha'}
    }
    if (url === '/api/admin/knowledge/raw/files' && options?.method === 'PUT') {
      writes.push({url,options})
      return {path:'raw/notes/Meeting.md',body:options.body?.body,sha256:'updated-sha'}
    }
    return original(url, options)
  })
  const wrapper=mount(KnowledgePage)
  try {
    await flushPromises()
    const fileTab = wrapper.findAll('[role="tablist"][aria-label="Knowledge source"] [role="tab"]')
      .find(tab => tab.attributes('aria-label') === 'File')!
    await fileTab.trigger('click')
    await flushPromises()
    await wrapper.get('button[aria-label="Create note in notes"]').trigger('click')
    const name=wrapper.get('input[aria-label="New note name"]')
    await name.setValue('Meeting.md')
    await name.trigger('keydown',{key:'Enter'})
    await flushPromises()
    expect(writes[0]).toEqual({url:'/api/admin/knowledge/raw/files',options:{
      method:'POST',body:{folder_path:'raw/notes',file_name:'Meeting.md',body:''},
    }})
    await wrapper.get('textarea#knowledge-note-body').setValue('Meeting notes')
    await wrapper.get('form.knowledge-note-editor').trigger('submit')
    await flushPromises()
    expect(writes[1]).toEqual({url:'/api/admin/knowledge/raw/files',options:{
      method:'PUT',body:{path:'raw/notes/Meeting.md',body:'Meeting notes',expected_sha256:'empty-sha'},
    }})
  } finally {wrapper.unmount()}
})

it('cancels inline note naming with Escape in the File tab', async () => {
  const wrapper=mount(KnowledgePage)
  try {
    await flushPromises()
    const fileTab = wrapper.findAll('[role="tablist"][aria-label="Knowledge source"] [role="tab"]')
      .find(tab => tab.attributes('aria-label') === 'File')!
    await fileTab.trigger('click')
    await flushPromises()
    await wrapper.get('button[aria-label="Add file"]').trigger('click')
    const name=wrapper.get('input[aria-label="New note name"]')
    await name.trigger('keydown',{key:'Escape'})
    expect(wrapper.find('input[aria-label="New note name"]').exists()).toBe(false)
  } finally {wrapper.unmount()}
})


it('creates nested folders through Explorer in the File tab without writing a file', async () => {
  const fallback = requestJsonMock.getMockImplementation()!
  const creates: unknown[] = []
  requestJsonMock.mockImplementation(async (url: string, options?: {method?: string;body?: unknown}) => {
    if (url === '/api/admin/knowledge/folders' && options?.method === 'POST') {
      creates.push(options.body)
      return { path: 'raw/docs/flow' }
    }
    return fallback(url)
  })
  const prompt = vi.spyOn(window, 'prompt').mockReturnValue('docs/flow')
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    const fileTab = wrapper.findAll('[role="tablist"][aria-label="Knowledge source"] [role="tab"]')
      .find(tab => tab.attributes('aria-label') === 'File')!
    await fileTab.trigger('click')
    await flushPromises()
    await wrapper.get('button[aria-label="Add folder"]').trigger('click')
    await flushPromises()
    expect(creates).toEqual([
      { space: 'raw', parent_path: 'raw', name: 'docs/flow' },
    ])
    expect(wrapper.find('input').exists()).toBe(false)
  } finally { prompt.mockRestore(); wrapper.unmount() }
})

it('remembers the expanded Knowledge folders and selected note on reload', async () => {
  const fallback = requestJsonMock.getMockImplementation()!
  requestJsonMock.mockImplementation(async (url: string) => {
    if (url === '/api/admin/knowledge/tree?space=wiki') {
      return { items: [
        { type: 'folder', space: 'wiki', path: 'wiki', name: 'wiki', editable: true },
        { type: 'folder', space: 'wiki', path: 'wiki/AI', name: 'AI', editable: true },
        { type: 'file', space: 'wiki', path: 'wiki/AI/LLM.md', name: 'LLM.md',
          id: 'wiki_llm', status: 'verified', revision: 1, editable: true },
        { type: 'file', space: 'wiki', path: 'wiki/wiki-file.md', name: 'wiki-file.md',
          id: 'wiki_demo', status: 'verified', revision: 1, editable: true },
      ] }
    }
    if (url === '/api/admin/knowledge/pages/wiki_llm') {
      return { id: 'wiki_llm', title: 'LLM', body: '# LLM', status: 'verified',
        revision: 1, source_ref: null }
    }
    return fallback(url)
  })

  const first = mount(KnowledgePage)
  await flushPromises()
  const folder = first.findAll('button.tree-folder').find(button => button.text().includes('AI'))
  expect(folder?.exists()).toBe(true)
  expect(folder!.attributes('aria-expanded')).toBe('true')
  await folder!.trigger('click')
  expect(folder!.attributes('aria-expanded')).toBe('false')
  await folder!.trigger('click')
  await flushPromises()
  expect(folder!.attributes('aria-expanded')).toBe('true')
  const file = first.findAll('button.tree-leaf').find(button => button.text().includes('LLM'))
  expect(file?.exists()).toBe(true)
  await file!.trigger('click')
  await flushPromises()
  expect(first.get('.doc h1').text()).toBe('LLM')
  expect(window.localStorage.getItem('voice-agent-knowledge-explorer-state-v1'))
    .toContain('wiki/AI/LLM.md')
  first.unmount()

  const second = mount(KnowledgePage)
  try {
    await flushPromises()
    const folderAgain = second.findAll('button.tree-folder')
      .find(button => button.text().includes('AI'))
    expect(folderAgain?.attributes('aria-expanded')).toBe('true')
    const fileAgain = second.findAll('button.tree-leaf')
      .find(button => button.text().includes('LLM'))
    expect(fileAgain?.attributes('aria-selected')).toBe('true')
    expect(second.get('.doc h1').text()).toBe('LLM')
  } finally { second.unmount() }
})

it('restores different Explorer folders when switching between Knowledge sources', async () => {
  const fallback = requestJsonMock.getMockImplementation()!
  requestJsonMock.mockImplementation(async (url: string) => {
    if (url.startsWith('/api/admin/knowledge/tree?')) {
      const space = new URL(url, 'http://localhost').searchParams.get('space')
      return { items: [
        { type: 'folder', space, path: space, name: space, editable: true },
        { type: 'folder', space, path: space + '/notes', name: 'notes', editable: true },
        { type: 'file', space, path: space + '/notes/inside.md', name: 'inside.md',
          id: 'wiki_demo', editable: true, revision: 1, status: 'verified' },
      ] }
    }
    return fallback(url)
  })
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    const tabs = wrapper.findAll('[role="tablist"][aria-label="Knowledge source"] [role="tab"]')
    let folder = wrapper.get('button.tree-folder')
    await folder.trigger('click')
    expect(folder.attributes('aria-expanded')).toBe('false')

    await tabs[1]!.trigger('click')
    await flushPromises()
    folder = wrapper.get('button.tree-folder')
    expect(folder.attributes('aria-expanded')).toBe('false')
    await folder.trigger('click')
    expect(folder.attributes('aria-expanded')).toBe('true')

    await tabs[0]!.trigger('click')
    await flushPromises()
    folder = wrapper.get('button.tree-folder')
    expect(folder.attributes('aria-expanded')).toBe('false')
  } finally { wrapper.unmount() }
})


it('renders the vault footer without a settings button', async () => {
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    const footer = wrapper.get('.a-vault-footer')
    expect(footer.text()).toContain('Local Markdown vault')
    expect(footer.find('button').exists()).toBe(false)
    expect(wrapper.find('.tree-holder').exists()).toBe(true)
  } finally {
    wrapper.unmount()
  }
})

it('opens a Job Wiki link even when the explorer previously selected raw space', async () => {
  window.localStorage.setItem('voice-agent-knowledge-vault-space', 'raw')
  routeQuery.wiki = 'wiki_demo'
  const wrapper = mount(KnowledgePage)
  await flushPromises()
  const knowledgeTab = wrapper.findAll('[role="tablist"][aria-label="Knowledge source"] [role="tab"]')
    .find(tab => tab.attributes('aria-label') === 'Knowledge')!
  expect(knowledgeTab.attributes('aria-selected')).toBe('true')
  expect(wrapper.get('.doc h1').text()).toBe('Wiki Demo')
  wrapper.unmount()
})

it('uploads a dropped Markdown file into the File space and displays its content', async () => {
  const original = requestJsonMock.getMockImplementation()!
  const writes: Array<{ url: string; options?: { method?: string; body?: Record<string, unknown> } }> = []
  requestJsonMock.mockImplementation(async (url: string, options?: { method?: string; body?: Record<string, unknown> }) => {
    if (url === '/api/admin/knowledge/tree?space=raw') return {
      items: [
        { type: 'folder', space: 'raw', path: 'raw', name: 'raw', editable: true },
        { type: 'folder', space: 'raw', path: 'raw/notes', name: 'notes', editable: true },
      ],
    }
    if (url === '/api/admin/knowledge/raw/files' && options?.method === 'POST') {
      writes.push({ url, options })
      return {
        path: String(options.body?.folder_path) + '/' + String(options.body?.file_name),
        body: String(options.body?.body ?? ''),
        sha256: 'dropped-sha',
      }
    }
    return original(url, options)
  })
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    const droppedFile = new File(['# Dropped Guide\n\nDragged into page.'], 'Guide.md', { type: 'text/markdown' })
    await wrapper.get('.knowledge-a').trigger('dragover', {
      dataTransfer: { types: ['Files'], files: [droppedFile] },
    })
    expect(wrapper.find('.a-drop-banner').exists()).toBe(true)

    await wrapper.get('.knowledge-a').trigger('drop', {
      dataTransfer: { types: ['Files'], files: [droppedFile] },
    })
    await flushPromises()
    expect(wrapper.find('.a-drop-banner').exists()).toBe(false)
    expect(writes).toEqual([{
      url: '/api/admin/knowledge/raw/files',
      options: {
        method: 'POST',
        body: { folder_path: 'raw', file_name: 'Guide.md', body: '# Dropped Guide\n\nDragged into page.' },
      },
    }])
    const fileTab = wrapper.findAll('[role="tablist"][aria-label="Knowledge source"] [role="tab"]')
      .find(tab => tab.attributes('aria-label') === 'File')!
    expect(fileTab.attributes('aria-selected')).toBe('true')
    expect(wrapper.get('.doc h1').text()).toBe('Guide')
    expect(wrapper.get('.markdown-body').text()).toContain('Dragged into page.')

    const folderFile = new File(['Folder note body'], 'Inside.md', { type: 'text/markdown' })
    await wrapper.get('.tree-folder-row').trigger('drop', {
      dataTransfer: { types: ['Files'], files: [folderFile] },
    })
    await flushPromises()
    expect(writes[1]).toEqual({
      url: '/api/admin/knowledge/raw/files',
      options: {
        method: 'POST',
        body: { folder_path: 'raw/notes', file_name: 'Inside.md', body: 'Folder note body' },
      },
    })
  } finally {
    wrapper.unmount()
  }
})

it('rejects non-Markdown files dropped onto the page', async () => {
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    const invalidFile = new File(['binary'], 'image.png', { type: 'image/png' })
    await wrapper.get('.knowledge-a').trigger('drop', {
      dataTransfer: { types: ['Files'], files: [invalidFile] },
    })
    await flushPromises()
    expect(wrapper.get('[role="alert"]').text()).toContain('Markdown')
    expect(requestJsonMock.mock.calls.some(([url, options]) =>
      url === '/api/admin/knowledge/raw/files' && options?.method === 'POST')).toBe(false)
  } finally {
    wrapper.unmount()
  }
})

it('shows shimmer placeholders in the explorer and note reader while loading', async () => {
  let resolveTree!: (value: { items: Array<Record<string, unknown>> }) => void
  let resolvePage!: (value: Record<string, unknown>) => void
  const fallback = requestJsonMock.getMockImplementation()!
  requestJsonMock.mockImplementation((url: string) => {
    if (url === '/api/admin/knowledge/tree?space=wiki') {
      return new Promise(resolve => { resolveTree = resolve })
    }
    if (url === '/api/admin/knowledge/pages/wiki_demo') {
      return new Promise(resolve => { resolvePage = resolve })
    }
    return fallback(url)
  })

  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    expect(wrapper.find('[data-knowledge-tree-skeleton]').exists()).toBe(true)
    expect(wrapper.find('[data-knowledge-detail-skeleton]').exists()).toBe(true)
    expect(wrapper.get('[data-knowledge-detail-skeleton]').attributes('aria-busy')).toBe('true')
    expect(wrapper.findAll('.card-shimmer').length).toBeGreaterThan(0)

    resolveTree({
      items: [
        { type: 'folder', space: 'wiki', path: 'wiki', name: 'wiki', editable: true },
        { type: 'file', space: 'wiki', path: 'wiki/wiki-file.md', name: 'wiki-file.md', id: 'wiki_demo', revision: 1, status: 'verified', editable: true },
      ],
    })
    await flushPromises()
    expect(wrapper.find('[data-knowledge-tree-skeleton]').exists()).toBe(false)
    expect(wrapper.find('[data-knowledge-detail-skeleton]').exists()).toBe(true)

    resolvePage({
      id: 'wiki_demo',
      title: 'Wiki Demo',
      body: '# Wiki Demo\n\nLoaded body.',
      status: 'verified',
      revision: 1,
      source_ref: null,
    })
    await flushPromises()
    expect(wrapper.find('[data-knowledge-detail-skeleton]').exists()).toBe(false)
    expect(wrapper.get('.doc h1').text()).toBe('Wiki Demo')
  } finally {
    wrapper.unmount()
  }
})

it('toggles Explorer search, queries Wiki and File spaces, and opens matching documents', async () => {
  const fallback = requestJsonMock.getMockImplementation()!
  requestJsonMock.mockImplementation(async (url: string) => {
    if (url === '/api/admin/knowledge/pages?q=Reciprocal+rank&space=wiki&mode=lex&limit=20') {
      return {
        items: [{
          id: 'wiki/concepts/hybrid-search.md',
          space: 'wiki',
          path: 'wiki/concepts/hybrid-search.md',
          title: 'Hybrid Search Architecture',
          revision: 1,
          status: 'verified',
          excerpt: 'Reciprocal rank fusion combines lexical BM25 and dense vectors.',
          match_line: 6,
          score: 0.92,
        }],
        next_cursor: null,
      }
    }
    if (url === '/api/admin/knowledge/pages/wiki%2Fconcepts%2Fhybrid-search.md') {
      return {
        id: 'wiki/concepts/hybrid-search.md',
        title: 'Hybrid Search Architecture',
        body: '# Hybrid Search Architecture\n\nReciprocal rank fusion details.',
        status: 'verified',
        revision: 1,
        source_ref: null,
      }
    }
    if (url === '/api/admin/knowledge/pages?q=Reciprocal+rank&space=raw&mode=lex&limit=20') {
      return {
        items: [],
        next_cursor: null,
      }
    }
    if (url === '/api/admin/knowledge/pages?q=barge-in&space=raw&mode=hybrid&limit=20') {
      return {
        items: [{
          id: 'sources/notes/field-observation.md',
          space: 'raw',
          path: 'raw/notes/field-observation.md',
          title: 'Field Observation',
          revision: 1,
          status: 'verified',
          excerpt: 'Acoustic barge-in latency was measured at 42 milliseconds.',
          match_line: 3,
          score: 0.88,
        }],
        next_cursor: null,
      }
    }
    if (url === '/api/admin/knowledge/raw/files?path=raw%2Fnotes%2Ffield-observation.md') {
      return {
        path: 'raw/notes/field-observation.md',
        body: '# Field Observation\n\nAcoustic barge-in latency was measured at 42 milliseconds.',
        sha256: 'obs-sha',
      }
    }
    return fallback(url)
  })

  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    expect(wrapper.find('input[aria-label="Search"]').exists()).toBe(false)

    // 1. Open search in Knowledge (wiki) space
    await wrapper.get('button[aria-label="Search"]').trigger('click')
    const input = wrapper.get('input[aria-label="Search"]')
    await input.setValue('Reciprocal rank')
    await wrapper.get('form.a-search-panel').trigger('submit')
    await flushPromises()

    expect(requestJsonMock).toHaveBeenCalledWith(
      '/api/admin/knowledge/pages?q=Reciprocal+rank&space=wiki&mode=lex&limit=20',
    )
    const wikiHit = wrapper.get('button.a-search-hit')
    expect(wikiHit.text()).toContain('Hybrid Search Architecture')
    expect(wikiHit.text()).toContain('Reciprocal rank fusion combines lexical BM25')
    expect(wikiHit.text()).toContain('L6')

    await wikiHit.trigger('click')
    await flushPromises()
    expect(wrapper.get('.doc h1').text()).toBe('Hybrid Search Architecture')
    expect(wrapper.get('.markdown-body').text()).toContain('Reciprocal rank fusion details.')

    // 2. Switching to File (raw) space keeps search open and automatically re-runs search in raw space
    const fileTab = wrapper.findAll('[role="tablist"][aria-label="Knowledge source"] [role="tab"]')
      .find(tab => tab.attributes('aria-label') === 'File')!
    await fileTab.trigger('click')
    await flushPromises()

    expect(wrapper.find('input[aria-label="Search"]').exists()).toBe(true)
    expect(requestJsonMock).toHaveBeenCalledWith(
      '/api/admin/knowledge/pages?q=Reciprocal+rank&space=raw&mode=lex&limit=20',
    )
    expect(wrapper.get('.a-search-empty').text()).toContain('No matching documents found.')

    await wrapper.get('select[aria-label="Search mode"]').setValue('hybrid')
    const rawInput = wrapper.get('input[aria-label="Search"]')
    await rawInput.setValue('barge-in')
    await wrapper.get('form.a-search-panel').trigger('submit')
    await flushPromises()

    expect(requestJsonMock).toHaveBeenCalledWith(
      '/api/admin/knowledge/pages?q=barge-in&space=raw&mode=hybrid&limit=20',
    )
    const rawHit = wrapper.get('button.a-search-hit')
    expect(rawHit.text()).toContain('Field Observation')
    expect(rawHit.text()).toContain('42 milliseconds')
    expect(rawHit.text()).toContain('L3')

    await rawHit.trigger('click')
    await flushPromises()
    expect(wrapper.get('.doc h1').text()).toContain('Field Observation')
    expect(wrapper.get('.markdown-body').text()).toContain('42 milliseconds')

    // 3. Pressing Escape closes the search input and restores the folder tree
    await wrapper.get('input[aria-label="Search"]').trigger('keydown', { key: 'Escape' })
    await flushPromises()
    expect(wrapper.find('input[aria-label="Search"]').exists()).toBe(false)
    expect(wrapper.find('.tree-holder').text()).toContain('raw-file')
  } finally {
    wrapper.unmount()
  }
})

it('shares a single pair of left and right panel toggle actions across Knowledge and Graph views and persists their states', async () => {
  const first = mount(KnowledgePage)
  try {
    await flushPromises()
    const explorer = first.get('aside.a-explorer')
    const outline = first.get('aside.a-outline')
    expect(explorer.classes()).not.toContain('is-collapsed')
    expect(outline.classes()).not.toContain('is-collapsed')
    expect(first.findAll('.a-outline-top button').map(button => button.attributes('role'))).toEqual(['tab', 'tab'])
    expect(first.find('button[aria-label="Immersive view"]').exists()).toBe(false)
    expect(first.find('button[aria-label="Split view unavailable"]').exists()).toBe(false)

    // Collapse both left (Explorer) and right (Outline & links) panels from .a-tab-actions
    await first.get('.a-tab-actions button[aria-label="Collapse explorer"]').trigger('click')
    await first.get('.a-tab-actions button[aria-label="Collapse outline & links"]').trigger('click')
    expect(explorer.classes()).toContain('is-collapsed')
    expect(outline.classes()).toContain('is-collapsed')
    expect(first.get('.knowledge-a').classes()).toContain('is-explorer-collapsed')
    expect(first.get('.knowledge-a').classes()).toContain('is-outline-collapsed')
    expect(window.localStorage.getItem('voice-agent-knowledge-explorer-collapsed')).toBe('true')
    expect(window.localStorage.getItem('voice-agent-knowledge-outline-collapsed')).toBe('true')

    // Switch to Graph view: the same 2 panel actions and shared right panel remain available
    await first.get('button.other-tab').trigger('click')
    await flushPromises()
    expect(first.find('aside.a-outline').exists()).toBe(false)
    expect(first.find('button[aria-label="Immersive view"]').exists()).toBe(false)
    expect(first.find('.a-tab-actions button[aria-label="Expand explorer"]').exists()).toBe(true)
    expect(first.find('.a-tab-actions button[aria-label="Expand chat"]').exists()).toBe(true)
  } finally {
    first.unmount()
  }

  // Remounting restores both collapsed states from localStorage
  const second = mount(KnowledgePage)
  try {
    await flushPromises()
    const explorer = second.get('aside.a-explorer')
    const outline = second.get('aside.a-outline')
    expect(explorer.classes()).toContain('is-collapsed')
    expect(outline.classes()).toContain('is-collapsed')

    // Expand both panels via the 2 shared actions in .a-tab-actions
    await second.get('.a-tab-actions button[aria-label="Expand explorer"]').trigger('click')
    await second.get('.a-tab-actions button[aria-label="Expand outline & links"]').trigger('click')

    expect(explorer.classes()).not.toContain('is-collapsed')
    expect(outline.classes()).not.toContain('is-collapsed')
    expect(window.localStorage.getItem('voice-agent-knowledge-explorer-collapsed')).toBe('false')
    expect(window.localStorage.getItem('voice-agent-knowledge-outline-collapsed')).toBe('false')
  } finally {
    second.unmount()
  }
})

it('renders YAML frontmatter as a collapsible Obsidian-style Properties section and removes raw frontmatter from the article body', async () => {
  const fallback = requestJsonMock.getMockImplementation()!
  requestJsonMock.mockImplementation(async (url: string) => {
    if (url === '/api/admin/knowledge/pages/wiki_demo') {
      return {
        id: 'wiki_demo',
        title: 'Đón không khí lạnh, miền Bắc giảm mấy độ?',
        body: [
          '---',
          'title: "Đón không khí lạnh, miền Bắc giảm mấy độ?"',
          'category: "source_summary"',
          'summary: "Tin ngắn cho biết không khí lạnh tác động miền Bắc."',
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
          '---',
          '',
          '# Đón không khí lạnh, miền Bắc giảm mấy độ?',
          '',
          'Đoạn mở đầu chính thức của bài viết.',
          '',
          '## Chi tiết dự báo',
          '',
          'Mưa rào và dông rải rác.',
        ].join('\n'),
        status: 'verified',
        revision: 1,
        source_ref: null,
      }
    }
    return fallback(url)
  })

  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    const propsSection = wrapper.get('section.doc-properties')
    expect(propsSection.text()).toContain('Properties')
    expect(propsSection.get('.doc-prop-badge').text()).toBe('source_summary')
    expect(propsSection.findAll('.doc-prop-tag').map(t => t.text())).toEqual([
      '#wiki/source',
      '#topic/weather/cold-air',
    ])
    expect(propsSection.get('.doc-prop-code').text()).toBe('don-khong-khi-lanh-mien-bac-giam-may-do.md')
    expect(propsSection.get('.doc-prop-date').text()).toBe('2026-10-24 17:17')
    expect(propsSection.text()).not.toContain('qmd')

    // Raw frontmatter is stripped from lead & markdown body, and duplicate H1 after frontmatter is removed
    expect(wrapper.get('.doc .lead').text()).toBe('Đoạn mở đầu chính thức của bài viết.')
    expect(wrapper.get('.markdown-body').text()).not.toContain('source_summary')
    expect(wrapper.find('.markdown-body h1').exists()).toBe(false)

    // Collapsing Properties hides the grid and persists in localStorage
    await propsSection.get('button.doc-properties-toggle').trigger('click')
    expect(propsSection.find('.doc-properties-grid').exists()).toBe(false)
    expect(window.localStorage.getItem('voice-agent-knowledge-properties-collapsed')).toBe('true')
  } finally {
    wrapper.unmount()
  }
})

it('renders a Recent section below the tree-folder panel and lists recently opened notes in most-recent-first order without duplicates', async () => {
  const fallback = requestJsonMock.getMockImplementation()!
  requestJsonMock.mockImplementation(async (url: string) => {
    if (url === '/api/admin/knowledge/tree?space=wiki') {
      return {
        items: [
          { type: 'folder', space: 'wiki', path: 'wiki', name: 'wiki', editable: true },
          { type: 'file', space: 'wiki', path: 'wiki/wiki-file.md', name: 'wiki-file.md', id: 'wiki_demo', revision: 1, status: 'verified', editable: true },
          { type: 'file', space: 'wiki', path: 'wiki/llm.md', name: 'llm.md', id: 'wiki_llm', revision: 1, status: 'verified', editable: true },
        ],
      }
    }
    if (url === '/api/admin/knowledge/pages/wiki_llm') {
      return {
        id: 'wiki_llm',
        title: 'LLM Guide',
        body: '# LLM Guide\n\nTransformer details.',
        status: 'verified',
        revision: 1,
        source_ref: null,
      }
    }
    return fallback(url)
  })

  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    const explorer = wrapper.get('aside.a-explorer')
    const explorerChildren = Array.from(explorer.element.children)
    const treeIndex = explorerChildren.findIndex(el => el.classList.contains('tree-holder'))
    const recentIndex = explorerChildren.findIndex(el => el.classList.contains('a-recent'))
    expect(treeIndex).toBeGreaterThanOrEqual(0)
    expect(recentIndex).toBe(treeIndex + 1)

    const recentSection = explorer.get('section.a-recent')
    expect(recentSection.text()).toContain('Recent')
    expect(recentSection.findAll('button.a-recent-item').map(btn => btn.get('.a-recent-title').text())).toEqual([
      'Wiki Demo',
    ])

    // Open LLM Guide -> moves to top of Recent
    const llmLeaf = wrapper.findAll('button.tree-leaf').find(btn => btn.text().includes('llm'))!
    await llmLeaf.trigger('click')
    await flushPromises()
    expect(recentSection.findAll('button.a-recent-item').map(btn => btn.get('.a-recent-title').text())).toEqual([
      'LLM Guide',
      'Wiki Demo',
    ])

    // Re-open Wiki Demo -> deduplicates and moves Wiki Demo back to top
    const demoLeaf = wrapper.findAll('button.tree-leaf').find(btn => btn.text().includes('wiki-file'))!
    await demoLeaf.trigger('click')
    await flushPromises()
    expect(recentSection.findAll('button.a-recent-item').map(btn => btn.get('.a-recent-title').text())).toEqual([
      'Wiki Demo',
      'LLM Guide',
    ])
  } finally {
    wrapper.unmount()
  }
})

it('keeps one Recent entry per file when using Split right', async () => {
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    expect(wrapper.findAll('button.a-recent-item')).toHaveLength(1)
    await wrapper.get('button[aria-label="Split right"]').trigger('click')
    await flushPromises()
    expect(wrapper.findAll('.doc h1').map(heading => heading.text())).toEqual(['Wiki Demo', 'Wiki Demo'])
    expect(wrapper.findAll('button.a-recent-item').map(item => item.get('.a-recent-title').text())).toEqual(['Wiki Demo'])

    const fileTab = wrapper.findAll('[aria-label="Knowledge source"] [role="tab"]')
      .find(tab => tab.attributes('aria-label') === 'File')!
    await fileTab.trigger('click')
    await flushPromises()
    await wrapper.findAll('button.tree-leaf').find(button => button.text().includes('raw-file'))!.trigger('click')
    await flushPromises()
    await wrapper.findAll('.a-pane')[1]!.get('button[aria-label="Split right"]').trigger('click')
    await flushPromises()
    expect(wrapper.findAll('button.a-recent-item').map(item => item.get('.a-recent-title').text())).toEqual(['raw-file', 'Wiki Demo'])
    expect(JSON.parse(window.localStorage.getItem('voice-agent-knowledge-recent-v1')!)).toHaveLength(2)
  } finally { wrapper.unmount() }
  const reloaded = mount(KnowledgePage)
  try {
    await flushPromises()
    expect(reloaded.findAll('button.a-recent-item').map(item => item.get('.a-recent-title').text())).toEqual(['raw-file', 'Wiki Demo'])
  } finally { reloaded.unmount() }
})

it('deduplicates Recent by path or stable ID on restore and reopening, while preserving distinct files', async () => {
  mockTwoWikiNotes()
  window.localStorage.setItem('voice-agent-knowledge-recent-v1', JSON.stringify([
    { space: 'wiki', path: 'wiki/alpha.md', title: 'Alpha', id: 'old-alpha' },
    { space: 'wiki', path: 'wiki/beta.md', title: 'Beta', id: 'beta' },
    { space: 'wiki', path: 'wiki/beta.md', title: 'Beta', id: 'old-beta' },
    { space: 'wiki', path: 'wiki/old-beta.md', title: 'Beta', id: 'beta' },
    { space: 'raw', path: 'raw/beta.md', title: 'Beta' },
    { space: 'wiki', path: 'wiki/concepts/beta.md', title: 'Beta', id: 'other-beta' },
  ]))
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    const paths = ['wiki/alpha.md', 'wiki/beta.md', 'raw/beta.md', 'wiki/concepts/beta.md']
    expect(wrapper.findAll('button.a-recent-item').map(item => item.attributes('title'))).toEqual(paths)
    const stored = JSON.parse(window.localStorage.getItem('voice-agent-knowledge-recent-v1')!)
    expect(stored.map((item: { path: string }) => item.path)).toEqual(paths)
    expect(stored[0].id).toBe('alpha')
    expect(stored[1].id).toBe('beta')
  } finally { wrapper.unmount() }
})

it('collapses and expands the Recent panel and persists its collapsed state in localStorage', async () => {
  const first = mount(KnowledgePage)
  try {
    await flushPromises()
    const recentSection = first.get('section.a-recent')
    const toggleBtn = recentSection.get('button.a-recent-toggle')
    expect(recentSection.classes()).not.toContain('is-collapsed')
    expect(toggleBtn.attributes('aria-expanded')).toBe('true')
    expect(recentSection.find('.a-recent-list').exists()).toBe(true)

    await toggleBtn.trigger('click')
    expect(recentSection.classes()).toContain('is-collapsed')
    expect(toggleBtn.attributes('aria-expanded')).toBe('false')
    expect(recentSection.find('.a-recent-list').exists()).toBe(false)
    expect(window.localStorage.getItem('voice-agent-knowledge-recent-collapsed')).toBe('true')
  } finally {
    first.unmount()
  }

  const second = mount(KnowledgePage)
  try {
    await flushPromises()
    const recentSection = second.get('section.a-recent')
    const toggleBtn = recentSection.get('button.a-recent-toggle')
    expect(recentSection.classes()).toContain('is-collapsed')
    expect(toggleBtn.attributes('aria-expanded')).toBe('false')
    expect(recentSection.find('.a-recent-list').exists()).toBe(false)

    await toggleBtn.trigger('click')
    expect(recentSection.classes()).not.toContain('is-collapsed')
    expect(toggleBtn.attributes('aria-expanded')).toBe('true')
    expect(recentSection.find('.a-recent-list').exists()).toBe(true)
    expect(window.localStorage.getItem('voice-agent-knowledge-recent-collapsed')).toBe('false')
  } finally {
    second.unmount()
  }
})

it('persists Recent items across reloads and switches space when clicking a Recent item from another tab', async () => {
  const first = mount(KnowledgePage)
  try {
    await flushPromises()
    // Initial load opened Wiki Demo in Knowledge (wiki) space
    const fileTab = first.findAll('[role="tablist"][aria-label="Knowledge source"] [role="tab"]')
      .find(tab => tab.attributes('aria-label') === 'File')!
    await fileTab.trigger('click')
    await flushPromises()
    const rawLeaf = first.findAll('button.tree-leaf').find(btn => btn.text().includes('raw-file'))!
    await rawLeaf.trigger('click')
    await flushPromises()
    expect(first.findAll('button.a-recent-item').map(btn => btn.get('.a-recent-title').text())).toEqual([
      'raw-file',
      'Wiki Demo',
    ])
  } finally {
    first.unmount()
  }

  const second = mount(KnowledgePage)
  try {
    await flushPromises()
    // Reloaded in File (raw) space: both raw-file and Wiki Demo remain in Recent
    const recentItems = second.findAll('button.a-recent-item')
    expect(recentItems.map(btn => btn.get('.a-recent-title').text())).toEqual([
      'raw-file',
      'Wiki Demo',
    ])

    // Click Wiki Demo in Recent while currently on File tab -> switches to Knowledge tab and opens Wiki Demo
    const wikiRecentBtn = recentItems.find(btn => btn.text().includes('Wiki Demo'))!
    await wikiRecentBtn.trigger('click')
    await flushPromises()

    const knowledgeTab = second.findAll('[role="tablist"][aria-label="Knowledge source"] [role="tab"]')
      .find(tab => tab.attributes('aria-label') === 'Knowledge')!
    expect(knowledgeTab.attributes('aria-selected')).toBe('true')
    expect(second.get('.doc h1').text()).toBe('Wiki Demo')
    expect(second.findAll('button.a-recent-item').map(btn => btn.get('.a-recent-title').text())).toEqual([
      'Wiki Demo',
      'raw-file',
    ])
  } finally {
    second.unmount()
  }
})

function mockTwoWikiNotes() {
  const base = requestJsonMock.getMockImplementation()!
  requestJsonMock.mockImplementation(async (url: string, init?: unknown) => {
    if (url === '/api/admin/knowledge/tree?space=wiki') return { items: [
      { type: 'folder', space: 'wiki', path: 'wiki', name: 'wiki', editable: true },
      { type: 'file', space: 'wiki', path: 'wiki/alpha.md', name: 'alpha.md', editable: true, id: 'alpha', revision: 1, status: 'verified' },
      { type: 'file', space: 'wiki', path: 'wiki/beta.md', name: 'beta.md', editable: true, id: 'beta', revision: 1, status: 'verified' },
    ] }
    if (url === '/api/admin/knowledge/pages/alpha') return { id: 'alpha', title: 'Alpha', body: 'Alpha body', status: 'verified', revision: 1, source_ref: null }
    if (url === '/api/admin/knowledge/pages/beta') return { id: 'beta', title: 'Beta', body: 'Beta body', status: 'verified', revision: 1, source_ref: null }
    return base(url, init)
  })
}
function openInTabButton(wrapper: ReturnType<typeof mount>, label: string, title: string) {
  return wrapper.get('[aria-label="' + label + ': ' + title + '"]')
}

it('opens an Explorer file in a new tab, keeps the first tab, and reuses a tab already open', async () => {
  mockTwoWikiNotes()
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    expect(wrapper.get('.doc h1').text()).toBe('Alpha')
    await openInTabButton(wrapper, 'Open in new tab', 'beta').trigger('click')
    await flushPromises()
    expect(wrapper.findAll('button.a-tab .tab-name').map(tab => tab.text())).toEqual(['Alpha', 'Beta'])
    expect(wrapper.get('.doc h1').text()).toBe('Beta')

    await wrapper.findAll('button.a-tab')[0]!.trigger('click')
    await flushPromises()
    expect(wrapper.get('.doc h1').text()).toBe('Alpha')

    await openInTabButton(wrapper, 'Open in new tab', 'beta').trigger('click')
    await flushPromises()
    expect(wrapper.findAll('button.a-tab')).toHaveLength(2)
    expect(wrapper.get('.doc h1').text()).toBe('Beta')

    await wrapper.findAll('.a-tab-close')[1]!.trigger('click')
    await flushPromises()
    expect(wrapper.findAll('button.a-tab .tab-name').map(tab => tab.text())).toEqual(['Alpha'])
    expect(wrapper.get('.doc h1').text()).toBe('Alpha')
  } finally { wrapper.unmount() }
})

it('opens with Ctrl+Click in a new tab and a plain click replaces the current tab', async () => {
  mockTwoWikiNotes()
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    const beta = wrapper.findAll('.tree-leaf').find(leaf => leaf.text() === 'beta')!
    await beta.trigger('click', { ctrlKey: true })
    await flushPromises()
    expect(wrapper.findAll('button.a-tab')).toHaveLength(2)

    await wrapper.findAll('.a-tab-close')[1]!.trigger('click')
    await flushPromises()
    await wrapper.findAll('.tree-leaf').find(leaf => leaf.text() === 'beta')!.trigger('click')
    await flushPromises()
    expect(wrapper.findAll('button.a-tab .tab-name').map(tab => tab.text())).toEqual(['Beta'])
  } finally { wrapper.unmount() }
})

it('shows files side by side and the outline follows the focused pane', async () => {
  mockTwoWikiNotes()
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    await openInTabButton(wrapper, 'Open to the side', 'beta').trigger('click')
    await flushPromises()
    expect(wrapper.findAll('.a-pane')).toHaveLength(2)
    expect(wrapper.findAll('.doc h1').map(heading => heading.text())).toEqual(['Alpha', 'Beta'])
    expect(wrapper.findAll('.a-pane')[1]!.classes()).toContain('is-focused')

    await wrapper.findAll('.a-pane')[0]!.trigger('pointerdown')
    await flushPromises()
    expect(wrapper.findAll('.a-pane')[0]!.classes()).toContain('is-focused')
    expect(wrapper.findAll('.doc h1').map(heading => heading.text())).toEqual(['Alpha', 'Beta'])

    await wrapper.findAll('.a-pane')[1]!.get('[aria-label="Close pane"]').trigger('click')
    await flushPromises()
    expect(wrapper.findAll('.a-pane')).toHaveLength(1)
    expect(wrapper.get('.doc h1').text()).toBe('Alpha')
  } finally { wrapper.unmount() }
})

it('opens a Graph node in a new tab and keeps Graph mounted to return to it', async () => {
  mockTwoWikiNotes()
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    await wrapper.get('button.other-tab').trigger('click')
    await flushPromises()
    const graph = wrapper.findComponent(KnowledgeGraph)
    graph.vm.$emit('open', 'beta')
    await flushPromises()
    expect(wrapper.findAll('button.a-tab .tab-name').map(tab => tab.text())).toEqual(['Alpha', 'Beta'])
    expect(wrapper.get('.doc h1').text()).toBe('Beta')
    expect(wrapper.findComponent(KnowledgeGraph).props('inactive')).toBe(true)
    expect(wrapper.get('.knowledge-a').classes()).not.toContain('is-graph-view')

    await wrapper.get('button.other-tab').trigger('click')
    await flushPromises()
    expect(wrapper.findComponent(KnowledgeGraph).vm).toBe(graph.vm)
    expect(wrapper.get('.knowledge-a').classes()).toContain('is-graph-view')
  } finally { wrapper.unmount() }
})

it('opens a Graph node to the side while Graph stays visible', async () => {
  mockTwoWikiNotes()
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    await wrapper.get('button.other-tab').trigger('click')
    await flushPromises()
    wrapper.findComponent(KnowledgeGraph).vm.$emit('open-side', 'beta')
    await flushPromises()
    expect(wrapper.findAll('.a-pane')).toHaveLength(2)
    expect(wrapper.findComponent(KnowledgeGraph).props('inactive')).toBe(false)
    expect(wrapper.findAll('.a-pane')[1]!.get('.doc h1').text()).toBe('Beta')
  } finally { wrapper.unmount() }
})

it('keeps a Knowledge tab when switching the Explorer to File and returns to it', async () => {
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    const sources = wrapper.findAll('.a-source-tab')
    await sources[1]!.trigger('click')
    await flushPromises()
    await wrapper.findAll('.tree-leaf')[0]!.trigger('click')
    await flushPromises()
    expect(wrapper.findAll('button.a-tab .tab-name').map(tab => tab.text())).toEqual(['Wiki Demo', 'raw-file'])
    expect(wrapper.get('.doc-overline').text()).toContain('raw')

    await sources[0]!.trigger('click')
    await flushPromises()
    expect(wrapper.get('.doc h1').text()).toBe('Wiki Demo')
    expect(wrapper.get('.doc-overline').text()).toContain('wiki')
    expect(wrapper.findAll('button.a-tab')).toHaveLength(2)
  } finally { wrapper.unmount() }
})

it('restores open tabs and panes after a reload', async () => {
  mockTwoWikiNotes()
  const first = mount(KnowledgePage)
  try {
    await flushPromises()
    await openInTabButton(first, 'Open to the side', 'beta').trigger('click')
    await flushPromises()
  } finally { first.unmount() }

  const second = mount(KnowledgePage)
  try {
    await flushPromises()
    expect(second.findAll('.a-pane')).toHaveLength(2)
    expect(second.findAll('.doc h1').map(heading => heading.text())).toEqual(['Alpha', 'Beta'])
  } finally { second.unmount() }
})

function mockPanelRects(wrapper: ReturnType<typeof mount>, widths: Record<string, number>) {
  for (const [selector, width] of Object.entries(widths)) {
    vi.spyOn(wrapper.get(selector).element, 'getBoundingClientRect').mockReturnValue({ width } as DOMRect)
  }
}

it('places the explorer toggle at the left of the tab bar and the right panel toggle at the right', async () => {
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    const bar = wrapper.get('.a-tabs').element
    const labels = (group: Element | null) => Array.from(group?.querySelectorAll('button') ?? [])
      .map(button => button.getAttribute('aria-label'))
    expect(labels(bar.firstElementChild)).toEqual(['Collapse explorer'])
    expect(labels(bar.lastElementChild)).toEqual(['Collapse outline & links'])
  } finally { wrapper.unmount() }
})

it('resizes the explorer and outline by dragging, clamps them, and remembers the widths', async () => {
  const first = mount(KnowledgePage)
  try {
    await flushPromises()
    mockPanelRects(first, { '.a-explorer': 258, '.a-outline': 320, '.a-workspace': 900 })

    first.get('.a-panel-resizer-explorer').element.dispatchEvent(new MouseEvent('pointerdown', { clientX: 258, bubbles: true }))
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 358 }))
    window.dispatchEvent(new MouseEvent('pointerup'))
    await flushPromises()
    expect((first.get('.a-explorer').element as HTMLElement).style.width).toBe('358px')

    // Short of the hide threshold, the panel snaps back to its minimum width.
    first.get('.a-panel-resizer-right').element.dispatchEvent(new MouseEvent('pointerdown', { clientX: 1000, bubbles: true }))
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 1100 }))
    window.dispatchEvent(new MouseEvent('pointerup'))
    await flushPromises()
    expect((first.get('.knowledge-a').element as HTMLElement).style.getPropertyValue('--knowledge-right-width')).toBe('240px')
  } finally { first.unmount() }

  const second = mount(KnowledgePage)
  try {
    await flushPromises()
    expect((second.get('.a-explorer').element as HTMLElement).style.width).toBe('358px')
    await second.get('[aria-label="Collapse explorer"]').trigger('click')
    expect((second.get('.a-explorer').element as HTMLElement).style.width).toBe('')
    expect(second.find('.a-panel-resizer-explorer').exists()).toBe(false)
  } finally { second.unmount() }
})

it('hides a panel dragged past its minimum width and restores its earlier width when reopened', async () => {
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    mockPanelRects(wrapper, { '.a-explorer': 300, '.a-workspace': 900 })
    wrapper.get('.a-panel-resizer-explorer').element.dispatchEvent(new MouseEvent('pointerdown', { clientX: 300, bubbles: true }))
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 100 }))
    await flushPromises()
    expect(wrapper.get('.knowledge-a').attributes('data-hiding-panel')).toBe('explorer')
    window.dispatchEvent(new MouseEvent('pointerup'))
    await flushPromises()
    expect(wrapper.get('.knowledge-a').classes()).toContain('is-explorer-collapsed')
    expect(wrapper.get('.knowledge-a').attributes('data-hiding-panel')).toBeUndefined()
    expect(window.localStorage.getItem('voice-agent-knowledge-explorer-collapsed')).toBe('true')

    await wrapper.get('[aria-label="Expand explorer"]').trigger('click')
    expect((wrapper.get('.a-explorer').element as HTMLElement).style.width).toBe('300px')
  } finally { wrapper.unmount() }
})

it('shows only Graph when no file is open and returns to reading when a file opens', async () => {
  mockTwoWikiNotes()
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    await wrapper.get('.a-tab-close').trigger('click')
    await flushPromises()
    expect(wrapper.find('button.a-tab').exists()).toBe(false)
    expect(wrapper.find('.doc').exists()).toBe(false)
    expect(wrapper.find('.a-pathbar').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('Select a Markdown file')
    expect(wrapper.get('.knowledge-a').classes()).toContain('is-graph-view')
    expect(wrapper.find('.mock-knowledge-graph').exists()).toBe(true)

    await wrapper.findAll('.tree-leaf').find(leaf => leaf.text() === 'beta')!.trigger('click')
    await flushPromises()
    expect(wrapper.get('.doc h1').text()).toBe('Beta')
    expect(wrapper.findAll('button.a-tab')).toHaveLength(1)
    expect(wrapper.get('.knowledge-a').classes()).not.toContain('is-graph-view')
  } finally { wrapper.unmount() }
})

it('opens an Explorer file in a new tab while Graph is showing instead of replacing the hidden note', async () => {
  mockTwoWikiNotes()
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    await wrapper.get('button.other-tab').trigger('click')
    await flushPromises()
    await wrapper.findAll('.tree-leaf').find(leaf => leaf.text() === 'beta')!.trigger('click')
    await flushPromises()
    expect(wrapper.findAll('button.a-tab .tab-name').map(tab => tab.text())).toEqual(['Alpha', 'Beta'])
    expect(wrapper.get('.doc h1').text()).toBe('Beta')
  } finally { wrapper.unmount() }
})

it('closes a pane left without a file, keeping the pane that hosts Graph', async () => {
  mockTwoWikiNotes()
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    await openInTabButton(wrapper, 'Open to the side', 'beta').trigger('click')
    await flushPromises()
    await wrapper.findAll('.a-pane')[1]!.get('.a-tab-close').trigger('click')
    await flushPromises()
    expect(wrapper.findAll('.a-pane')).toHaveLength(1)
    expect(wrapper.get('.doc h1').text()).toBe('Alpha')
  } finally { wrapper.unmount() }
})

it('stays on Graph after a reload when every file was closed', async () => {
  mockTwoWikiNotes()
  const first = mount(KnowledgePage)
  try {
    await flushPromises()
    await first.get('.a-tab-close').trigger('click')
    await flushPromises()
  } finally { first.unmount() }

  const second = mount(KnowledgePage)
  try {
    await flushPromises()
    expect(second.find('.doc').exists()).toBe(false)
    expect(second.get('.knowledge-a').classes()).toContain('is-graph-view')
  } finally { second.unmount() }
})

it('offers the Wiki chat beside a note and switches the right panel between outline and chat', async () => {
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    expect(wrapper.find('.wiki-chat-panel').exists()).toBe(false)
    const tabs = wrapper.findAll('.a-outline .a-right-tab')
    expect(tabs.map(tab => tab.attributes('aria-selected'))).toEqual(['true', 'false'])
    await tabs[1]!.trigger('click')
    expect(wrapper.find('.a-outline').exists()).toBe(false)
    expect(wrapper.find('.wiki-chat-panel textarea').exists()).toBe(true)
    expect(wrapper.find('.doc h1').exists()).toBe(true)

    await wrapper.get('.wiki-chat-panel textarea').setValue('Kept draft')
    await wrapper.findAll('.wiki-chat-panel .a-right-tab')[0]!.trigger('click')
    expect(wrapper.find('.a-outline').exists()).toBe(true)
    expect((wrapper.get('.wiki-chat-panel').element as HTMLElement).style.display).toBe('none')
    await wrapper.findAll('.a-outline .a-right-tab')[1]!.trigger('click')
    expect((wrapper.get('.wiki-chat-panel textarea').element as HTMLTextAreaElement).value).toBe('Kept draft')
  } finally { wrapper.unmount() }
})

it('keeps the chat beside a note opened from Graph', async () => {
  mockTwoWikiNotes()
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    await wrapper.get('button.other-tab').trigger('click')
    await flushPromises()
    expect(wrapper.find('.wiki-chat-panel .a-right-tab').exists()).toBe(false)
    wrapper.findComponent(KnowledgeGraph).vm.$emit('open', 'beta')
    await flushPromises()
    expect(wrapper.get('.doc h1').text()).toBe('Beta')
    expect(wrapper.find('.a-outline').exists()).toBe(true)
  } finally { wrapper.unmount() }
})

it('gives outline and chat one right panel width and one collapsed state', async () => {
  window.localStorage.setItem('voice-agent-knowledge-panel-widths-v1', JSON.stringify({ outline: 300, chat: 450 }))
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    const root = wrapper.get('.knowledge-a')
    const width = () => (root.element as HTMLElement).style.getPropertyValue('--knowledge-right-width')
    expect(width()).toBe('450px')

    mockPanelRects(wrapper, { '.a-outline': 450, '.a-workspace': 900 })
    await wrapper.get('.a-panel-resizer-right').trigger('keydown', { key: 'ArrowRight' })
    expect(width()).toBe('434px')

    await wrapper.findAll('.a-outline .a-right-tab')[1]!.trigger('click')
    expect(width()).toBe('434px')
    expect(wrapper.findAll('.a-panel-resizer-right')).toHaveLength(1)

    await wrapper.get('[aria-label="Collapse chat"]').trigger('click')
    expect(wrapper.get('.wiki-chat-panel').classes()).toContain('is-collapsed')
    expect(wrapper.find('.a-panel-resizer-right').exists()).toBe(false)
    expect(window.localStorage.getItem('voice-agent-knowledge-outline-collapsed')).toBe('true')
  } finally { wrapper.unmount() }
})


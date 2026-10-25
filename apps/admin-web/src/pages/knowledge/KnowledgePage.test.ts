import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, it, vi } from 'vitest'
import { setLocale } from '@/composables/useI18n'
import KnowledgePage from './KnowledgePage.vue'

const { requestJsonMock, navigateMock, routeQuery } = vi.hoisted(() => ({ requestJsonMock: vi.fn(), navigateMock: vi.fn(), routeQuery: {} as Record<string,string> }))
vi.mock('@/api/client', () => ({
  requestJson: requestJsonMock,
  jsonRequest: (method: string, body: unknown) => ({ method, body }),
  withQuery: (path: string, query: Record<string, string | number>) =>
    path + '?' + new URLSearchParams(
      Object.entries(query).map(([key, value]) => [key, String(value)]),
    ).toString(),
}))
vi.mock('vue-router', () => ({ useRouter: () => ({ push: navigateMock }), useRoute: () => ({ query: routeQuery }) }))
vi.mock('./KnowledgeGraph.vue', () => ({ default: { template: '<div>Knowledge graph</div>' } }))

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

it('defaults to Knowledge and switches its single displayed tree using the source tabs', async () => {
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    expect(wrapper.find('select[aria-label="Knowledge source"]').exists()).toBe(false)
    expect(wrapper.get('.a-pane-heading').text()).not.toContain('File explorer')
    const tablist = wrapper.get('.a-pane-heading [role="tablist"][aria-label="Knowledge source"]')
    const tabs = tablist.findAll('[role="tab"]')
    expect(tabs.map(tab => tab.text())).toEqual(['Knowledge', 'File'])
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
    expect(wrapper.find('textarea').exists()).toBe(false)
  } finally { wrapper.unmount() }
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
      .find(tab => tab.text() === 'File')!
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
    .find(tab => tab.text() === 'File')!
  await fileTab.trigger('click')
  await flushPromises()
  expect(window.localStorage.getItem('voice-agent-knowledge-vault-space')).toBe('raw')
  first.unmount()
  requestJsonMock.mockClear()
  const again = mount(KnowledgePage)
  try {
    await flushPromises()
    const restoredFileTab = again.findAll('[role="tablist"][aria-label="Knowledge source"] [role="tab"]')
      .find(tab => tab.text() === 'File')!
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
      .find(tab => tab.text() === 'File')!
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
      .find(tab => tab.text() === 'File')!
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
      .find(tab => tab.text() === 'File')!
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


it('navigates to Knowledge settings from the gear in the vault footer', async () => {
  const wrapper = mount(KnowledgePage)
  try {
    await flushPromises()
    const footer = wrapper.get('.a-vault-footer')
    const settings = footer.get('button[aria-label="Knowledge settings"]')
    expect(footer.text()).toContain('Local Markdown vault')
    await settings.trigger('click')
    expect(navigateMock).toHaveBeenCalledExactlyOnceWith('/knowledge/settings')
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
    .find(tab => tab.text() === 'Knowledge')!
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
      .find(tab => tab.text() === 'File')!
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

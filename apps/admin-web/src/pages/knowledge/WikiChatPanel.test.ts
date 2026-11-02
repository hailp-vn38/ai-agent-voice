import { mount } from '@vue/test-utils'
import { beforeEach, expect, it, vi } from 'vitest'
import { setLocale } from '@/composables/useI18n'
import { renderMermaidBlocks } from './mermaidRenderer'
import WikiChatPanel from './WikiChatPanel.vue'
import type { WikiChatMessage } from './useWikiChat'
import type { WikiConversationSummary } from '@/api/wiki-chat'

function assistant(status: WikiChatMessage['status']): WikiChatMessage {
  return {
    id: 'a1', role: 'assistant', text: 'The answer.', status, tools: [], graph: null,
    citations: [], errorCode: null,
  }
}

vi.mock('./mermaidRenderer', () => ({ renderMermaidBlocks: vi.fn(() => Promise.resolve()) }))

beforeEach(() => {
  setLocale('en')
  vi.mocked(renderMermaidBlocks).mockClear()
})

it('renders Mermaid only after the answer finishes and keeps its source out of the announcement', async () => {
  const text = 'Before.\n\n```mermaid\nflowchart TD\n  A --> B\n```\n\nAfter.'
  const wrapper = mount(WikiChatPanel, { props: { messages: [{ ...assistant('streaming'), text }], busy: true } })
  try {
    await wrapper.vm.$nextTick()
    expect(wrapper.find('pre.knowledge-mermaid').exists()).toBe(true)
    expect(renderMermaidBlocks).not.toHaveBeenCalled()

    await wrapper.setProps({ messages: [{ ...assistant('done'), text }], busy: false })
    expect(renderMermaidBlocks).toHaveBeenCalledTimes(1)
    expect(vi.mocked(renderMermaidBlocks).mock.calls[0]![0]).toBe(wrapper.get('.wiki-chat-answer').element)
    const announcement = wrapper.get('.wiki-chat-announcement').text()
    expect(announcement).toContain('Before.')
    expect(announcement).not.toContain('flowchart')
  } finally { wrapper.unmount() }
})

it('does not load Mermaid for an answer without a diagram', async () => {
  const wrapper = mount(WikiChatPanel, { props: { messages: [assistant('done')], busy: false } })
  try {
    await wrapper.vm.$nextTick()
    expect(renderMermaidBlocks).not.toHaveBeenCalled()
  } finally { wrapper.unmount() }
})

it('announces an answer once it completes, not while it streams', async () => {
  const wrapper = mount(WikiChatPanel, { props: { messages: [assistant('streaming')], busy: true } })
  try {
    const region = wrapper.get('.wiki-chat-announcement')
    expect(region.attributes('aria-live')).toBe('polite')
    expect(region.text()).toBe('')
    expect(wrapper.get('.wiki-chat-answer').attributes('aria-live')).toBeUndefined()

    await wrapper.setProps({ messages: [assistant('done')], busy: false })
    expect(wrapper.get('.wiki-chat-announcement').text()).toBe('The answer.')
  } finally { wrapper.unmount() }
})

it('names the speakers only for screen readers and keeps the question and answer visible', () => {
  const user: WikiChatMessage = { ...assistant('done'), id: 'u1', role: 'user', text: 'The question' }
  const wrapper = mount(WikiChatPanel, { props: { messages: [user, assistant('done')], busy: false } })
  try {
    const text = wrapper.get('.wiki-chat-messages').text()
    expect(text).toContain('The question')
    expect(text).toContain('The answer.')
    expect(text).not.toContain('You')
    expect(text).not.toContain('Wiki assistant')
    expect(wrapper.get('.wiki-chat-message.is-user').attributes('aria-label')).toBe('You')
    expect(wrapper.get('.wiki-chat-message.is-assistant').attributes('aria-label')).toBe('Wiki assistant')
  } finally { wrapper.unmount() }
})

it('collapses completed tools into one trail and shows a running tool on its own', () => {
  const message: WikiChatMessage = {
    ...assistant('streaming'),
    tools: [
      { call_id: 'c1', name: 'wiki_query', state: 'completed', query: 'FVG', pages: ['wiki/fvg.md'] },
      { call_id: 'c2', name: 'wiki_get', state: 'running', query: 'wiki/fvg.md', pages: [] },
    ],
  }
  const wrapper = mount(WikiChatPanel, { props: { messages: [message], busy: true } })
  try {
    expect(wrapper.get('.wiki-chat-trail > summary').text()).toBe('Research steps · 1')
    expect(wrapper.get('.wiki-chat-trail').attributes('open')).toBeUndefined()
    expect(wrapper.findAll('.wiki-chat-tool-summary')).toHaveLength(1)
    expect(wrapper.get('.wiki-chat-tools').text()).toContain('Read page')
  } finally { wrapper.unmount() }
})

it('shows a thinking state until the first text or running tool arrives', async () => {
  const waiting: WikiChatMessage = { ...assistant('streaming'), text: '' }
  const wrapper = mount(WikiChatPanel, { props: { messages: [waiting], busy: true } })
  try {
    expect(wrapper.find('.wiki-chat-thinking').exists()).toBe(true)
    expect(wrapper.find('.wiki-chat-answer').exists()).toBe(false)
    await wrapper.setProps({ messages: [assistant('streaming')] })
    expect(wrapper.find('.wiki-chat-thinking').exists()).toBe(false)
  } finally { wrapper.unmount() }
})

it('offers the open file and lets attached files be removed', async () => {
  const file = { path: 'wiki/known.md', title: 'Known' }
  const wrapper = mount(WikiChatPanel, { props: { messages: [], busy: false, openFile: file } })
  try {
    await wrapper.get('.wiki-chat-file-add').trigger('click')
    expect(wrapper.emitted('pinFile')).toEqual([[file]])

    await wrapper.setProps({ pinnedFiles: [file] })
    expect(wrapper.find('.wiki-chat-file-add').exists()).toBe(false)
    expect(wrapper.get('.wiki-chat-files .wiki-chat-file').text()).toBe('Known')
    await wrapper.get('.wiki-chat-file-remove').trigger('click')
    expect(wrapper.emitted('unpinFile')).toEqual([['wiki/known.md']])
  } finally { wrapper.unmount() }
})

it('keeps the composer free of file chips without an open file and blocks a fifth one', async () => {
  const wrapper = mount(WikiChatPanel, { props: { messages: [], busy: false, openFile: null } })
  try {
    expect(wrapper.find('.wiki-chat-files').exists()).toBe(false)
    const pinnedFiles = ['a', 'b', 'c', 'd'].map(name => ({ path: `wiki/${name}.md`, title: name }))
    await wrapper.setProps({ pinnedFiles, openFile: { path: 'wiki/e.md', title: 'e' } })
    const add = wrapper.get('.wiki-chat-file-add')
    expect(add.attributes('disabled')).toBeDefined()
    expect(add.attributes('title')).toBe('You can attach up to 4 files')
  } finally { wrapper.unmount() }
})

it('shows which files a question was asked about', () => {
  const user: WikiChatMessage = {
    ...assistant('done'), id: 'u1', role: 'user', text: 'Summarize this',
    files: [{ path: 'wiki/known.md', title: 'Known' }],
  }
  const wrapper = mount(WikiChatPanel, { props: { messages: [user], busy: false } })
  try {
    expect(wrapper.get('.wiki-chat-user-files').text()).toBe('Known')
  } finally { wrapper.unmount() }
})

it('opens the saved conversation list, selects a conversation, renames, confirms deletion, and starts a new one', async () => {
  const conversations: WikiConversationSummary[] = [
    { id: 'current', title: 'Current topic', updated_at_ms: 20, message_count: 4, running: false },
    { id: 'older', title: 'Older topic', updated_at_ms: 10, message_count: 2, running: false },
  ]
  const wrapper = mount(WikiChatPanel, {
    props: {
      messages: [], busy: false, conversations, conversationId: 'current',
    },
  })
  try {
    await wrapper.get('.wiki-chat-header-action').trigger('click')
    expect(wrapper.emitted('loadConversations')).toHaveLength(1)
    expect(wrapper.get('.wiki-chat-conversation-list').text()).toContain('Older topic')

    const older = wrapper.findAll('.wiki-chat-conversation-item').find(item => item.text().includes('Older topic'))!
    await older.get('.wiki-chat-conversation-select').trigger('click')
    expect(wrapper.emitted('selectConversation')).toEqual([['older']])

    await wrapper.get('.wiki-chat-header-action').trigger('click')
    const current = wrapper.findAll('.wiki-chat-conversation-item').find(item => item.text().includes('Current topic'))!
    await current.get('[aria-label="Rename"]').trigger('click')
    const titleInput = current.get('input')
    await titleInput.setValue('Renamed topic')
    await current.get('form').trigger('submit')
    expect(wrapper.emitted('renameConversation')).toEqual([['current', 'Renamed topic']])

    await current.get('[aria-label="Delete"]').trigger('click')
    const dialog = wrapper.findComponent({ name: 'ConfirmDialog' })
    expect(dialog.props('modelValue')).toBe(true)
    expect(wrapper.emitted('deleteConversation')).toBeUndefined()
    dialog.vm.$emit('confirm')
    expect(wrapper.emitted('deleteConversation')).toEqual([['current']])

    await wrapper.get('.wiki-chat-header-action:last-child').trigger('click')
    expect(wrapper.emitted('newConversation')).toHaveLength(1)
  } finally { wrapper.unmount() }
})

it('prevents switching to another conversation or starting a new one while an answer is running', async () => {
  const conversations: WikiConversationSummary[] = [
    { id: 'current', title: 'Current topic', updated_at_ms: 20, message_count: 2, running: true },
    { id: 'older', title: 'Older topic', updated_at_ms: 10, message_count: 2, running: false },
  ]
  const wrapper = mount(WikiChatPanel, {
    props: { messages: [assistant('streaming')], busy: true, conversations, conversationId: 'current' },
  })
  try {
    const newConversation = wrapper.get('.wiki-chat-header-action:last-child')
    expect(newConversation.attributes('disabled')).toBeDefined()
    await wrapper.get('.wiki-chat-header-action:first-child').trigger('click')
    const select = wrapper.findAll('.wiki-chat-conversation-select').find(button => button.text().includes('Older topic'))!
    expect(select.attributes('disabled')).toBeDefined()
    expect(wrapper.findAll('.wiki-chat-conversation-actions button').at(1)?.attributes('disabled')).toBeDefined()
  } finally { wrapper.unmount() }
})

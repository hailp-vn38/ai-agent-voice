import { mount } from '@vue/test-utils'
import { beforeEach, expect, it } from 'vitest'
import { setLocale } from '@/composables/useI18n'
import WikiChatPanel from './WikiChatPanel.vue'
import type { WikiChatMessage } from './useWikiChat'

function assistant(status: WikiChatMessage['status']): WikiChatMessage {
  return {
    id: 'a1', role: 'assistant', text: 'The answer.', status, tools: [], graph: null,
    citations: [], errorCode: null,
  }
}

beforeEach(() => setLocale('en'))

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

import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, it, vi } from 'vitest'
import { setLocale } from '@/composables/useI18n'
import KnowledgePage from './KnowledgePage.vue'
import { KNOWLEDGE_BACKEND_KEY, MemoryKnowledgeBackend } from './knowledgeBackend'

vi.mock('vue-router', () => ({
  useRoute: () => ({ query: {} }),
  useRouter: () => ({ replace: vi.fn().mockResolvedValue(undefined) }),
}))
vi.mock('./useWikiChat', async () => {
  const { ref } = await import('vue')
  const noop = vi.fn()
  return {
    useWikiChat: () => ({
      messages: ref([]), conversationId: ref(null), loadingConversation: ref(false), conversations: ref([]),
      loadingConversations: ref(false), conversationListError: ref(''), readOnlyConversation: ref(false),
      conversationLoadFailed: ref(false), activeGraph: ref(null), busy: ref(false), chatDrawerOpen: ref(false),
      pinnedFiles: ref([]), pinFile: noop, unpinFile: noop, send: noop, stop: noop, retry: noop, reset: noop,
      loadConversation: async () => 'not_found', refreshConversations: noop, renameConversation: noop,
      deleteConversation: async () => false, refreshConversation: noop, showGraphOf: noop,
    }),
  }
})
vi.mock('./WikiChatPanel.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return { default: defineComponent({ setup: () => () => h('div', { class: 'chat-isolated-for-vault-test' }) }) }
})

beforeEach(() => {
  window.localStorage.clear()
  window.localStorage.setItem('voice-agent-knowledge-vault-space', 'wiki')
  setLocale('en')
})

function mountKnowledgePage(backend = new MemoryKnowledgeBackend({
    pages: [
      { id: 'wiki/gardening.md', path: 'wiki/gardening.md', title: 'Gardening', body: '# Gardening\n\nTrồng cà chua trong nhà kính.', revision: 1 },
      { id: 'wiki/compost.md', path: 'wiki/compost.md', title: 'Compost', body: '# Compost\n\nTurn leaves into soil.', revision: 1 },
    ],
    rawFiles: [{ path: 'raw/field-note.md', body: '# Field Note\n\nObserved tomatoes.', sha256: 'raw-rev-1' }],
  })) {
  const wrapper = mount(KnowledgePage, {
    global: { provide: { [KNOWLEDGE_BACKEND_KEY as symbol]: backend } },
  })
  return { wrapper, backend }
}

it('loads and opens a Wiki Page through MemoryKnowledgeBackend without HTTP fetch mocks', async () => {
  const fetchSpy = vi.spyOn(globalThis, 'fetch')
  const { wrapper } = mountKnowledgePage()
  try {
    await flushPromises()
    expect(wrapper.get('.doc h1').text()).toBe('Gardening')
    expect(wrapper.get('.markdown-body').text()).toContain('Trồng cà chua trong nhà kính.')
    expect(fetchSpy).not.toHaveBeenCalled()
  } finally {
    wrapper.unmount()
    fetchSpy.mockRestore()
  }
})

it('loads a Raw Working Note through MemoryKnowledgeBackend', async () => {
  window.localStorage.setItem('voice-agent-knowledge-vault-space', 'raw')
  const { wrapper } = mountKnowledgePage()
  try {
    await flushPromises()
    expect(wrapper.get('.doc h1').text()).toBe('field-note')
    expect(wrapper.get('.markdown-body').text()).toContain('Observed tomatoes.')
  } finally { wrapper.unmount() }
})

it('searches without Vietnamese diacritics and opens the matching page through MemoryKnowledgeBackend', async () => {
  const fetchSpy = vi.spyOn(globalThis, 'fetch')
  const { wrapper } = mountKnowledgePage()
  try {
    await flushPromises()
    await wrapper.get('button[aria-label="Search"]').trigger('click')
    await wrapper.get('input[aria-label="Search"]').setValue('ca chua')
    await wrapper.get('form.a-search-panel').trigger('submit')
    await flushPromises()
    expect(wrapper.get('.a-search-hit').text()).toContain('Gardening')
    await wrapper.get('.a-search-hit').trigger('click')
    await flushPromises()
    expect(wrapper.get('.doc h1').text()).toBe('Gardening')
    expect(fetchSpy).not.toHaveBeenCalled()
  } finally { wrapper.unmount(); fetchSpy.mockRestore() }
})

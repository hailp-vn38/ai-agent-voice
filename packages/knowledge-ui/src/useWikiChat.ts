import { ref } from 'vue'

export type WikiChatFile = { path: string; title: string }

/** Optional chat integration point. The desktop package has no chat adapter in v1. */
export function useWikiChat() {
  const messages = ref<unknown[]>([])
  const conversationId = ref<string | null>(null)
  const loadingConversation = ref(false)
  const conversations = ref<unknown[]>([])
  const loadingConversations = ref(false)
  const conversationListError = ref('')
  const readOnlyConversation = ref(false)
  const conversationLoadFailed = ref(false)
  const activeGraph = ref<null>(null)
  const busy = ref(false)
  const chatDrawerOpen = ref(false)
  const pinnedFiles = ref<WikiChatFile[]>([])
  const noop = async (..._args: unknown[]) => undefined
  return {
    messages, conversationId, loadingConversation, conversations, loadingConversations,
    conversationListError, readOnlyConversation, conversationLoadFailed, activeGraph, busy,
    chatDrawerOpen, pinnedFiles, pinFile: noop, unpinFile: noop, send: noop, stop: noop,
    retry: noop, reset: () => { messages.value = []; conversationId.value = null },
    loadConversation: noop, refreshConversations: noop, renameConversation: noop,
    deleteConversation: async (..._args: unknown[]) => false, refreshConversation: noop, showGraphOf: (..._args: unknown[]) => undefined,
  }
}

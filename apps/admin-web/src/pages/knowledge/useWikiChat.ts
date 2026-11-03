import { computed, onBeforeUnmount, ref } from 'vue'
import {
  askWiki,
  deleteWikiConversation,
  getWikiConversation,
  getWikiPageTitle,
  listWikiConversations,
  renameWikiConversation,
  WikiChatRequestError,
  type AnswerGraph,
  type WikiChatAskInput,
  type WikiConversationDetail,
  type WikiConversationSummary,
  type WikiChatTool,
} from '@/api/wiki-chat'
import { isApiError } from '@/api/errors'

/** A vault file the user attached to the conversation; `path` is a `wiki/` or `sources/` path. */
export interface WikiChatFile {
  path: string
  title: string
}

export interface WikiChatMessage {
  id: string
  role: 'user' | 'assistant'
  text: string
  status: 'streaming' | 'done' | 'stopped' | 'failed' | 'interrupted'
  tools: WikiChatTool[]
  graph: AnswerGraph | null
  citations: string[]
  errorCode: string | null
  saved?: boolean
  contentTruncated?: boolean
  files?: WikiChatFile[]
}

export const MAX_PINNED_FILES = 4

export type ConversationLoadResult = 'loaded' | 'not_found' | 'failed'

function messageFromSaved(message: WikiConversationDetail['messages'][number]): WikiChatMessage {
  const files = message.pinned_paths.map(path => ({ path, title: path }))
  return {
    id: message.id,
    role: message.role,
    text: message.content,
    status: message.status === 'done' ? 'done'
      : message.status === 'stopped' ? 'stopped'
        : message.status === 'interrupted' ? 'interrupted'
          : message.status === 'failed' ? 'failed' : 'streaming',
    tools: message.tools,
    graph: message.graph,
    citations: message.citations,
    errorCode: message.error_code,
    saved: true,
    contentTruncated: message.content_truncated,
    ...(files.length ? { files } : {}),
  }
}

export function useWikiChat() {
  const messages = ref<WikiChatMessage[]>([])
  const activeGraph = ref<AnswerGraph | null>(null)
  const activeGraphMessageId = ref<string | null>(null)
  const busy = ref(false)
  const chatDrawerOpen = ref(false)
  const pinnedFiles = ref<WikiChatFile[]>([])
  const conversationId = ref<string | null>(null)
  const loadingConversation = ref(false)
  const conversations = ref<WikiConversationSummary[]>([])
  const loadingConversations = ref(false)
  const conversationListError = ref<string | null>(null)
  const readOnlyConversation = ref(false)
  const conversationLoadFailed = ref(false)
  let controller: AbortController | null = null
  let currentAssistantId: string | null = null

  const hasMessages = computed(() => messages.value.length > 0)

  function findMessage(id: string): WikiChatMessage | undefined {
    return messages.value.find(message => message.id === id)
  }

  function pinFile(file: WikiChatFile) {
    if (readOnlyConversation.value || pinnedFiles.value.length >= MAX_PINNED_FILES) return
    if (pinnedFiles.value.some(pinned => pinned.path === file.path)) return
    pinnedFiles.value.push(file)
  }

  function unpinFile(path: string) {
    if (readOnlyConversation.value) return
    pinnedFiles.value = pinnedFiles.value.filter(file => file.path !== path)
  }

  async function loadConversation(id: string): Promise<ConversationLoadResult> {
    loadingConversation.value = true
    try {
      const conversation = await getWikiConversation(id)
      conversationId.value = conversation.id
      messages.value = conversation.messages.map(messageFromSaved)
      readOnlyConversation.value = conversation.messages.some(message => message.status === 'running')
      conversationLoadFailed.value = false
      const latestUser = [...conversation.messages].reverse().find(message => message.role === 'user')
      pinnedFiles.value = latestUser?.pinned_paths.map(path => ({ path, title: path })) ?? []
      const pinnedPaths = [...new Set(conversation.messages.flatMap(message => message.pinned_paths))]
      const titles = new Map<string, string>()
      let nextPath = 0
      await Promise.all(Array.from({ length: Math.min(8, pinnedPaths.length) }, async () => {
        while (nextPath < pinnedPaths.length) {
          const path = pinnedPaths[nextPath++]!
          if (!path.startsWith('wiki/') && !path.startsWith('sources/')) continue
          try {
            titles.set(path, await getWikiPageTitle(path))
          } catch {
            // Keep the durable path visible if a page was renamed or deleted.
          }
        }
      }))
      for (const message of messages.value) {
        if (!message.files) continue
        message.files = message.files.map(file => ({ ...file, title: titles.get(file.path) ?? file.path }))
      }
      pinnedFiles.value = pinnedFiles.value.map(file => ({ ...file, title: titles.get(file.path) ?? file.path }))
      const graphMessage = [...messages.value].reverse().find(message => message.role === 'assistant' && message.graph?.nodes.length)
      activeGraph.value = graphMessage?.graph ?? null
      activeGraphMessageId.value = graphMessage?.id ?? null
      chatDrawerOpen.value = true
      return 'loaded'
    } catch (error) {
      if (isApiError(error) && error.status === 404) {
        reset()
        return 'not_found'
      }
      // The conversation may still exist, so keep its id and offer a reload instead of forgetting it.
      if (conversationId.value !== id) {
        reset()
        conversationId.value = id
      }
      readOnlyConversation.value = true
      conversationLoadFailed.value = true
      chatDrawerOpen.value = true
      return 'failed'
    } finally {
      loadingConversation.value = false
    }
  }

  async function refreshConversations(): Promise<void> {
    loadingConversations.value = true
    conversationListError.value = null
    try {
      conversations.value = await listWikiConversations()
    } catch {
      conversationListError.value = 'conversation_list_failed'
    } finally {
      loadingConversations.value = false
    }
  }

  async function renameConversation(id: string, title: string): Promise<boolean> {
    conversationListError.value = null
    try {
      await renameWikiConversation(id, title)
      await refreshConversations()
      return true
    } catch (error) {
      conversationListError.value = isApiError(error) ? error.code : 'conversation_rename_failed'
      return false
    }
  }

  async function deleteConversation(id: string): Promise<boolean> {
    if (busy.value) {
      conversationListError.value = 'wiki_conversation_busy'
      return false
    }
    conversationListError.value = null
    try {
      await deleteWikiConversation(id)
      if (conversationId.value === id) reset()
      await refreshConversations()
      return true
    } catch (error) {
      conversationListError.value = isApiError(error) ? error.code : 'conversation_delete_failed'
      return false
    }
  }

  async function runRequest(input: WikiChatAskInput, assistantId: string) {
    const assistant = findMessage(assistantId)
    if (!assistant) return
    const requestController = new AbortController()
    controller = requestController
    currentAssistantId = assistantId
    busy.value = true
    assistant.status = 'streaming'
    assistant.errorCode = null
    let lastGraphRevision: number | null = null
    try {
      await askWiki(input, {
        onTurn: turn => {
          conversationId.value = turn.conversation_id
          readOnlyConversation.value = false
          const turnIndex = messages.value.findIndex(message => message.id === assistantId)
          if (turnIndex > 0 && messages.value[turnIndex - 1]?.role === 'user') {
            messages.value[turnIndex - 1]!.id = turn.user_message_id
          }
          assistant.id = turn.assistant_message_id
          assistantId = turn.assistant_message_id
          currentAssistantId = turn.assistant_message_id
        },
        onTool: tool => {
          const current = findMessage(assistantId)
          if (!current) return
          const index = current.tools.findIndex(item => item.call_id === tool.call_id)
          if (index < 0) current.tools.push(tool)
          else current.tools[index] = tool
        },
        onGraph: graph => {
          const current = findMessage(assistantId)
          // The server resends the last snapshot at the end of a turn; redrawing
          // it would reset the Graph the user may already be exploring.
          if (!current || graph.revision === lastGraphRevision) return
          lastGraphRevision = graph.revision
          current.graph = graph
          if (graph.nodes.length > 0) {
            activeGraph.value = graph
            activeGraphMessageId.value = assistantId
            chatDrawerOpen.value = true
          }
        },
        onDelta: text => {
          const current = findMessage(assistantId)
          if (current) current.text += text
        },
        onDone: result => {
          const current = findMessage(assistantId)
          if (!current) return
          current.citations = result.citations
          current.saved = result.saved
          current.contentTruncated = result.content_truncated
          if (!result.graph_saved) {
            current.graph = null
            if (activeGraphMessageId.value === current.id) {
              activeGraph.value = null
              activeGraphMessageId.value = null
            }
          }
          if (current.status === 'streaming') {
            if (result.stop_reason === 'finished') current.status = 'done'
            else if (result.stop_reason === 'stopped') current.status = 'stopped'
            else {
              current.status = 'failed'
              current.errorCode = result.stop_reason
            }
          }
        },
        onError: code => {
          const current = findMessage(assistantId)
          if (current) {
            current.status = 'failed'
            current.errorCode = code
          }
        },
      }, requestController.signal)
      const current = findMessage(assistantId)
      if (current?.status === 'streaming') current.status = 'done'
    } catch (error) {
      const current = findMessage(assistantId)
      if (current) {
        if (requestController.signal.aborted) current.status = 'stopped'
        else {
          current.status = 'failed'
          current.errorCode = error instanceof WikiChatRequestError ? error.code : 'wiki_chat_connection_lost'
        }
      }
    } finally {
      if (controller === requestController) controller = null
      if (currentAssistantId === assistantId) currentAssistantId = null
      busy.value = false
    }
  }

  async function send(text: string) {
    const content = text.trim()
    if (!content || busy.value || loadingConversation.value || readOnlyConversation.value) return
    chatDrawerOpen.value = true
    const files = [...pinnedFiles.value]
    const user: WikiChatMessage = {
      id: `local:${Date.now()}:user`, role: 'user', text: content, status: 'done', tools: [], graph: null,
      citations: [], errorCode: null, files,
    }
    const assistant: WikiChatMessage = {
      id: `local:${Date.now()}:assistant`, role: 'assistant', text: '', status: 'streaming', tools: [], graph: null,
      citations: [], errorCode: null,
    }
    messages.value.push(user, assistant)
    const input: WikiChatAskInput = {
      ...(conversationId.value ? { conversation_id: conversationId.value } : {}),
      question: content,
      pinned_paths: files.map(file => file.path),
    }
    await runRequest(input, assistant.id)
  }

  function stop() {
    if (!controller) return
    const assistant = currentAssistantId ? findMessage(currentAssistantId) : undefined
    if (assistant?.status === 'streaming') assistant.status = 'stopped'
    controller.abort()
  }

  function clearAnswer(assistant: WikiChatMessage) {
    assistant.text = ''
    assistant.tools = []
    assistant.citations = []
    assistant.errorCode = null
    assistant.graph = null
    delete assistant.saved
    delete assistant.contentTruncated
    if (activeGraphMessageId.value === assistant.id) {
      activeGraph.value = null
      activeGraphMessageId.value = null
    }
  }

  function retry(assistantId: string) {
    if (busy.value || readOnlyConversation.value) return
    const assistantIndex = messages.value.findIndex(message => message.id === assistantId)
    const assistant = messages.value[assistantIndex]
    const previousUser = assistantIndex > 0 ? messages.value[assistantIndex - 1] : undefined
    if (!assistant || assistant.role !== 'assistant' || !previousUser || previousUser.role !== 'user') return
    if (assistantIndex !== messages.value.length - 1) return
    if (assistant.status !== 'failed' && assistant.status !== 'interrupted') return
    clearAnswer(assistant)
    // A local id means the server rejected the request before saving a turn, so there is nothing to retry by id.
    if (assistant.id.startsWith('local:')) {
      void runRequest({
        ...(conversationId.value ? { conversation_id: conversationId.value } : {}),
        question: previousUser.text,
        pinned_paths: (previousUser.files ?? []).map(file => file.path),
      }, assistantId)
      return
    }
    if (!conversationId.value) return
    void runRequest({ conversation_id: conversationId.value, retry_message_id: assistantId }, assistantId)
  }

  function refreshConversation() {
    if (!conversationId.value || busy.value || loadingConversation.value) return
    void loadConversation(conversationId.value)
  }

  function showGraphOf(id: string) {
    const message = findMessage(id)
    if (!message?.graph || message.graph.nodes.length === 0) return
    activeGraph.value = message.graph
    activeGraphMessageId.value = id
  }

  function reset() {
    stop()
    messages.value = []
    conversationId.value = null
    readOnlyConversation.value = false
    conversationLoadFailed.value = false
    pinnedFiles.value = []
    activeGraph.value = null
    activeGraphMessageId.value = null
    chatDrawerOpen.value = false
  }

  onBeforeUnmount(stop)

  return {
    messages,
    conversationId,
    loadingConversation,
    conversations,
    loadingConversations,
    conversationListError,
    readOnlyConversation,
    conversationLoadFailed,
    activeGraph,
    activeGraphMessageId,
    busy,
    hasMessages,
    chatDrawerOpen,
    pinnedFiles,
    pinFile,
    unpinFile,
    send,
    loadConversation,
    refreshConversations,
    renameConversation,
    deleteConversation,
    refreshConversation,
    stop,
    retry,
    reset,
    showGraphOf,
  }
}

import { computed, onBeforeUnmount, ref } from 'vue'
import {
  askWiki,
  getWikiConversation,
  WikiChatRequestError,
  type AnswerGraph,
  type WikiChatAskInput,
  type WikiConversationDetail,
  type WikiChatTool,
} from '@/api/wiki-chat'

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
  const readOnlyConversation = ref(false)
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

  async function loadConversation(id: string): Promise<boolean> {
    loadingConversation.value = true
    try {
      const conversation = await getWikiConversation(id)
      conversationId.value = conversation.id
      messages.value = conversation.messages.map(messageFromSaved)
      readOnlyConversation.value = conversation.messages.some(message => message.status === 'running')
      const latestUser = [...conversation.messages].reverse().find(message => message.role === 'user')
      pinnedFiles.value = latestUser?.pinned_paths.map(path => ({ path, title: path })) ?? []
      const graphMessage = [...messages.value].reverse().find(message => message.role === 'assistant' && message.graph)
      activeGraph.value = graphMessage?.graph ?? null
      activeGraphMessageId.value = graphMessage?.id ?? null
      chatDrawerOpen.value = true
      return true
    } catch {
      reset()
      return false
    } finally {
      loadingConversation.value = false
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

  function retry(assistantId: string) {
    if (busy.value || readOnlyConversation.value) return
    const assistantIndex = messages.value.findIndex(message => message.id === assistantId)
    const assistant = messages.value[assistantIndex]
    const previousUser = assistantIndex > 0 ? messages.value[assistantIndex - 1] : undefined
    if (!assistant || assistant.role !== 'assistant' || !previousUser || previousUser.role !== 'user') return
    if (assistantIndex !== messages.value.length - 1) return
    if (assistant.id.startsWith('local:') && assistant.errorCode === 'wiki_conversation_busy') {
      assistant.text = ''
      assistant.tools = []
      assistant.citations = []
      assistant.errorCode = null
      assistant.status = 'streaming'
      void runRequest({
        ...(conversationId.value ? { conversation_id: conversationId.value } : {}),
        question: previousUser.text,
        pinned_paths: (previousUser.files ?? []).map(file => file.path),
      }, assistantId)
      return
    }
    if (assistant.status !== 'failed' && assistant.status !== 'interrupted') return
    if (!conversationId.value) return
    assistant.text = ''
    assistant.tools = []
    assistant.citations = []
    assistant.errorCode = null
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
    readOnlyConversation,
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
    refreshConversation,
    stop,
    retry,
    reset,
    showGraphOf,
  }
}

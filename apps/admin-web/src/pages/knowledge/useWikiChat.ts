import { computed, onBeforeUnmount, ref } from 'vue'
import {
  askWiki,
  WikiChatRequestError,
  type AnswerGraph,
  type WikiChatInputMessage,
  type WikiChatTool,
} from '@/api/wiki-chat'

export interface WikiChatMessage {
  id: string
  role: 'user' | 'assistant'
  text: string
  status: 'streaming' | 'done' | 'stopped' | 'failed'
  tools: WikiChatTool[]
  graph: AnswerGraph | null
  citations: string[]
  errorCode: string | null
}

function newId(): string {
  return typeof crypto !== 'undefined' && 'randomUUID' in crypto
    ? crypto.randomUUID()
    : `${Date.now()}-${Math.random().toString(36).slice(2)}`
}

const MAX_HISTORY_MESSAGES = 12
const MAX_HISTORY_BYTES = 16 * 1024

function historyFor(messages: WikiChatMessage[]): WikiChatInputMessage[] {
  return messages
    .filter(message => message.status === 'done' && message.text.trim())
    .map(message => ({ role: message.role, content: message.text }))
}

// Mirrors the server limits, keeping the newest messages. The question itself
// is always sent, so one that is too long still gets the server's invalid_input.
function boundHistory(history: WikiChatInputMessage[]): WikiChatInputMessage[] {
  const encoder = new TextEncoder()
  let start = history.length - 1
  let bytes = encoder.encode(history[start]?.content ?? '').length
  while (start > 0 && history.length - start < MAX_HISTORY_MESSAGES) {
    bytes += encoder.encode(history[start - 1]!.content).length
    if (bytes > MAX_HISTORY_BYTES) break
    start -= 1
  }
  while (start < history.length - 1 && history[start]!.role === 'assistant') start += 1
  return history.slice(start)
}

export function useWikiChat() {
  const messages = ref<WikiChatMessage[]>([])
  const activeGraph = ref<AnswerGraph | null>(null)
  const activeGraphMessageId = ref<string | null>(null)
  const busy = ref(false)
  const chatDrawerOpen = ref(false)
  let controller: AbortController | null = null
  let currentAssistantId: string | null = null

  const hasMessages = computed(() => messages.value.length > 0)

  function findMessage(id: string): WikiChatMessage | undefined {
    return messages.value.find(message => message.id === id)
  }

  async function runRequest(history: WikiChatInputMessage[], assistantId: string) {
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
      await askWiki(history, {
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
          if (current.status === 'streaming') {
            if (result.stop_reason === 'finished') current.status = 'done'
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
    if (!content || busy.value) return
    chatDrawerOpen.value = true
    const history = historyFor(messages.value)
    const user: WikiChatMessage = {
      id: newId(), role: 'user', text: content, status: 'done', tools: [], graph: null,
      citations: [], errorCode: null,
    }
    const assistant: WikiChatMessage = {
      id: newId(), role: 'assistant', text: '', status: 'streaming', tools: [], graph: null,
      citations: [], errorCode: null,
    }
    messages.value.push(user, assistant)
    history.push({ role: 'user', content })
    await runRequest(boundHistory(history), assistant.id)
  }

  function stop() {
    if (!controller) return
    const assistant = currentAssistantId ? findMessage(currentAssistantId) : undefined
    if (assistant?.status === 'streaming') assistant.status = 'stopped'
    controller.abort()
  }

  function retry(assistantId: string) {
    if (busy.value) return
    const assistantIndex = messages.value.findIndex(message => message.id === assistantId)
    const assistant = messages.value[assistantIndex]
    const previousUser = assistantIndex > 0 ? messages.value[assistantIndex - 1] : undefined
    if (!assistant || assistant.role !== 'assistant' || !previousUser || previousUser.role !== 'user') return
    const history = historyFor(messages.value.slice(0, assistantIndex - 1))
    history.push({ role: 'user', content: previousUser.text })
    assistant.text = ''
    assistant.tools = []
    assistant.citations = []
    assistant.errorCode = null
    void runRequest(boundHistory(history), assistantId)
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
    activeGraph.value = null
    activeGraphMessageId.value = null
    chatDrawerOpen.value = false
  }

  onBeforeUnmount(stop)

  return {
    messages,
    activeGraph,
    activeGraphMessageId,
    busy,
    hasMessages,
    chatDrawerOpen,
    send,
    stop,
    retry,
    reset,
    showGraphOf,
  }
}

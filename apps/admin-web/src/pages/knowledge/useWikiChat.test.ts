import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, it, vi } from 'vitest'
import { defineComponent } from 'vue'
import { WikiChatRequestError, type AnswerGraph, type WikiChatAskInput, type WikiConversationDetail } from '@/api/wiki-chat'
import { useWikiChat } from './useWikiChat'

const { askWikiMock, getWikiConversationMock } = vi.hoisted(() => ({
  askWikiMock: vi.fn(),
  getWikiConversationMock: vi.fn(),
}))
vi.mock('@/api/wiki-chat', async importOriginal => ({
  ...await importOriginal<typeof import('@/api/wiki-chat')>(),
  askWiki: askWikiMock,
  getWikiConversation: getWikiConversationMock,
}))

type Chat = ReturnType<typeof useWikiChat>

function mountChat(): Chat {
  let chat: Chat | undefined
  mount(defineComponent({
    setup() {
      chat = useWikiChat()
      return () => null
    },
  }))
  return chat!
}

let nextTurn = 0
function answerWith(
  text: string,
  graphs: AnswerGraph[] = [],
  contentTruncated = false,
  graphSaved = true,
) {
  askWikiMock.mockImplementationOnce(async (input: WikiChatAskInput, handlers) => {
    nextTurn += 1
    handlers.onTurn({
      conversation_id: input.conversation_id ?? 'conversation-1',
      user_message_id: `user-${nextTurn}`,
      assistant_message_id: `assistant-${nextTurn}`,
    })
    graphs.forEach(graph => handlers.onGraph(graph))
    handlers.onDelta(text)
    handlers.onDone({
      citations: [], stop_reason: 'finished', saved: true, content_truncated: contentTruncated,
      graph_saved: graphSaved,
    })
  })
}

function graphWith(revision: number, title: string): AnswerGraph {
  return {
    revision,
    nodes: [{ id: title, path: title, title, status: 'verified', revision: 1, tier: 'cited' }],
    edges: [],
    unresolved: [],
    truncated: false,
  } as AnswerGraph
}

function lastInput(): WikiChatAskInput {
  return askWikiMock.mock.lastCall![0]
}

beforeEach(() => {
  askWikiMock.mockReset()
  getWikiConversationMock.mockReset()
  nextTurn = 0
})

it('sends only the question and then uses the server conversation id for follow-ups', async () => {
  const chat = mountChat()
  answerWith('first answer')
  await chat.send('first question')
  expect(lastInput()).toEqual({ question: 'first question', pinned_paths: [] })
  expect(chat.conversationId.value).toBe('conversation-1')
  expect(chat.messages.value.map(message => message.id)).toEqual(['user-1', 'assistant-1'])
  expect(chat.messages.value[1]?.text).toBe('first answer')

  answerWith('second answer')
  await chat.send('follow-up')
  expect(lastInput()).toEqual({
    conversation_id: 'conversation-1', question: 'follow-up', pinned_paths: [],
  })
  expect(JSON.stringify(lastInput())).not.toContain('messages')
})

it('continues applying graph and delta events after the server turn ids replace local ids', async () => {
  const chat = mountChat()
  answerWith('answer text', [graphWith(1, 'answer graph')])
  await chat.send('question')
  const assistant = chat.messages.value[1]!
  expect(assistant.id).toBe('assistant-1')
  expect(assistant.text).toBe('answer text')
  expect(assistant.graph?.nodes[0]?.title).toBe('answer graph')
  expect(chat.activeGraphMessageId.value).toBe('assistant-1')
  expect(assistant.status).toBe('done')
})

it('restores saved messages, citations, graph and the latest question pinned files', async () => {
  const graph = graphWith(3, 'saved graph')
  getWikiConversationMock.mockResolvedValue({
    id: 'conversation-9', title: 'Saved topic', created_at_ms: 1, updated_at_ms: 2,
    messages: [
      {
        id: 'user-9', seq: 1, role: 'user', content: 'Saved question', content_truncated: false,
        status: 'done', error_code: null, pinned_paths: ['wiki/a.md'], tools: [], citations: [],
        graph: null, created_at_ms: 1, updated_at_ms: 1,
      },
      {
        id: 'assistant-9', seq: 2, role: 'assistant', content: 'Saved answer', content_truncated: true,
        status: 'done', error_code: null, pinned_paths: [], tools: [], citations: ['wiki/a.md'],
        graph, created_at_ms: 1, updated_at_ms: 2,
      },
    ],
  } satisfies WikiConversationDetail)

  const chat = mountChat()
  expect(await chat.loadConversation('conversation-9')).toBe(true)
  expect(chat.conversationId.value).toBe('conversation-9')
  expect(chat.messages.value[1]?.text).toBe('Saved answer')
  expect(chat.messages.value[1]?.contentTruncated).toBe(true)
  expect(chat.messages.value[1]?.citations).toEqual(['wiki/a.md'])
  expect(chat.activeGraph.value?.revision).toBe(3)
  expect(chat.pinnedFiles.value).toEqual([{ path: 'wiki/a.md', title: 'wiki/a.md' }])
})

it('keeps a conversation with a running answer read-only until the user reloads it', async () => {
  getWikiConversationMock.mockResolvedValue({
    id: 'conversation-running', title: 'Working', created_at_ms: 1, updated_at_ms: 2,
    messages: [
      {
        id: 'user-running', seq: 1, role: 'user', content: 'Question in another tab', content_truncated: false,
        status: 'done', error_code: null, pinned_paths: [], tools: [], citations: [], graph: null,
        created_at_ms: 1, updated_at_ms: 1,
      },
      {
        id: 'assistant-running', seq: 2, role: 'assistant', content: 'Partial answer', content_truncated: false,
        status: 'running', error_code: null, pinned_paths: [], tools: [], citations: [], graph: null,
        created_at_ms: 1, updated_at_ms: 2,
      },
    ],
  } satisfies WikiConversationDetail)
  const chat = mountChat()
  expect(await chat.loadConversation('conversation-running')).toBe(true)
  expect(chat.readOnlyConversation.value).toBe(true)
  await chat.send('do not send')
  expect(askWikiMock).not.toHaveBeenCalled()
  chat.refreshConversation()
  await flushPromises()
  expect(getWikiConversationMock).toHaveBeenCalledTimes(2)
  expect(askWikiMock).not.toHaveBeenCalled()
})

it('does not auto-resend a busy question and retries it only after the user asks', async () => {
  const chat = mountChat()
  answerWith('previous answer')
  await chat.send('first question')
  askWikiMock.mockRejectedValueOnce(new WikiChatRequestError('wiki_conversation_busy'))
  await chat.send('follow-up')
  expect(askWikiMock).toHaveBeenCalledTimes(2)
  const localAssistant = chat.messages.value.at(-1)!
  expect(localAssistant.status).toBe('failed')
  expect(localAssistant.errorCode).toBe('wiki_conversation_busy')

  answerWith('follow-up answer')
  chat.retry(localAssistant.id)
  await flushPromises()
  expect(askWikiMock).toHaveBeenCalledTimes(3)
  expect(lastInput()).toEqual({
    conversation_id: 'conversation-1', question: 'follow-up', pinned_paths: [],
  })
  expect(chat.messages.value.at(-1)?.text).toBe('follow-up answer')
})

it('keeps the live storage truncation result on the assistant message', async () => {
  const chat = mountChat()
  answerWith('shortened answer', [], true)
  await chat.send('question')
  expect(chat.messages.value[1]?.contentTruncated).toBe(true)
})

it('hides a live graph that exceeded the server storage limit', async () => {
  const chat = mountChat()
  answerWith('answer', [graphWith(1, 'large graph')], false, false)
  await chat.send('question')
  expect(chat.messages.value[1]?.graph).toBeNull()
  expect(chat.activeGraph.value).toBeNull()
  expect(chat.activeGraphMessageId.value).toBeNull()
})

it('sends the pinned paths and retains the question files after server ids arrive', async () => {
  const chat = mountChat()
  chat.pinFile({ path: 'wiki/a.md', title: 'A' })
  chat.pinFile({ path: 'wiki/a.md', title: 'A again' })
  chat.pinFile({ path: 'sources/b.md', title: 'B' })
  answerWith('first')
  await chat.send('one')
  expect(lastInput()).toEqual({
    question: 'one', pinned_paths: ['wiki/a.md', 'sources/b.md'],
  })
  expect(chat.messages.value[0]?.files?.map(file => file.title)).toEqual(['A', 'B'])

  chat.unpinFile('wiki/a.md')
  answerWith('second')
  await chat.send('two')
  expect(lastInput()).toEqual({
    conversation_id: 'conversation-1', question: 'two', pinned_paths: ['sources/b.md'],
  })
  expect(chat.messages.value[0]?.files).toHaveLength(2)
})

it('sends server retry ids and keeps retry files attached', async () => {
  const chat = mountChat()
  for (const name of ['a', 'b', 'c', 'd', 'e']) chat.pinFile({ path: `wiki/${name}.md`, title: name })
  expect(chat.pinnedFiles.value.map(file => file.title)).toEqual(['a', 'b', 'c', 'd'])

  askWikiMock.mockImplementationOnce(async (input: WikiChatAskInput, handlers) => {
    handlers.onTurn({
      conversation_id: input.conversation_id ?? 'conversation-1',
      user_message_id: 'user-1', assistant_message_id: 'assistant-1',
    })
    handlers.onError('provider_failed')
  })
  await chat.send('question')
  chat.unpinFile('wiki/a.md')
  answerWith('recovered')
  chat.retry('assistant-1')
  await flushPromises()
  expect(lastInput()).toEqual({ conversation_id: 'conversation-1', retry_message_id: 'assistant-1' })

  chat.reset()
  expect(chat.pinnedFiles.value).toEqual([])
})

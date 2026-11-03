import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, it, vi } from 'vitest'
import { defineComponent } from 'vue'
import { ApiError } from '@/api/errors'
import { WikiChatRequestError, type AnswerGraph, type WikiChatAskInput, type WikiConversationDetail } from '@/api/wiki-chat'
import { useWikiChat } from './useWikiChat'

const { askWikiMock, getWikiConversationMock, getWikiPageTitleMock } = vi.hoisted(() => ({
  askWikiMock: vi.fn(),
  getWikiConversationMock: vi.fn(),
  getWikiPageTitleMock: vi.fn(),
}))
vi.mock('@/api/wiki-chat', async importOriginal => ({
  ...await importOriginal<typeof import('@/api/wiki-chat')>(),
  askWiki: askWikiMock,
  getWikiConversation: getWikiConversationMock,
  getWikiPageTitle: getWikiPageTitleMock,
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
  getWikiPageTitleMock.mockReset()
  getWikiPageTitleMock.mockImplementation(async (path: string) => path)
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
        id: 'user-8', seq: 1, role: 'user', content: 'Earlier question', content_truncated: false,
        status: 'done', error_code: null, pinned_paths: ['wiki/old.md'], tools: [], citations: [],
        graph: null, created_at_ms: 1, updated_at_ms: 1,
      },
      {
        id: 'assistant-8', seq: 2, role: 'assistant', content: 'Earlier answer', content_truncated: false,
        status: 'done', error_code: null, pinned_paths: [], tools: [], citations: [],
        graph: null, created_at_ms: 1, updated_at_ms: 1,
      },
      {
        id: 'user-9', seq: 3, role: 'user', content: 'Saved question', content_truncated: false,
        status: 'done', error_code: null, pinned_paths: ['wiki/a.md', 'sources/b/original.md'], tools: [], citations: [],
        graph: null, created_at_ms: 1, updated_at_ms: 1,
      },
      {
        id: 'assistant-9', seq: 4, role: 'assistant', content: 'Saved answer', content_truncated: true,
        status: 'done', error_code: null, pinned_paths: [], tools: [], citations: ['wiki/a.md'],
        graph, created_at_ms: 1, updated_at_ms: 2,
      },
    ],
  } satisfies WikiConversationDetail)

  getWikiPageTitleMock.mockResolvedValue('Current title')
  const chat = mountChat()
  expect(await chat.loadConversation('conversation-9')).toBe('loaded')
  expect(chat.conversationId.value).toBe('conversation-9')
  expect(chat.messages.value[3]?.text).toBe('Saved answer')
  expect(chat.messages.value[3]?.contentTruncated).toBe(true)
  expect(chat.messages.value[3]?.citations).toEqual(['wiki/a.md'])
  expect(chat.activeGraph.value?.revision).toBe(3)
  expect(chat.pinnedFiles.value).toEqual([
    { path: 'wiki/a.md', title: 'Current title' },
    { path: 'sources/b/original.md', title: 'Current title' },
  ])
  expect(getWikiPageTitleMock).toHaveBeenCalledWith('sources/b/original.md')
  expect(chat.messages.value[0]?.files).toEqual([{ path: 'wiki/old.md', title: 'Current title' }])
  expect(chat.messages.value[2]?.files).toEqual([
    { path: 'wiki/a.md', title: 'Current title' },
    { path: 'sources/b/original.md', title: 'Current title' },
  ])
})

it('falls back to the stored Wiki path when its current title can no longer be loaded', async () => {
  getWikiConversationMock.mockResolvedValue({
    id: 'conversation-10', title: 'Saved topic', created_at_ms: 1, updated_at_ms: 2,
    messages: [{
      id: 'user-10', seq: 1, role: 'user', content: 'Question', content_truncated: false,
      status: 'done', error_code: null, pinned_paths: ['wiki/gone.md'], tools: [], citations: [],
      graph: null, created_at_ms: 1, updated_at_ms: 1,
    }],
  } satisfies WikiConversationDetail)
  getWikiPageTitleMock.mockRejectedValue(new Error('page deleted'))

  const chat = mountChat()
  await chat.loadConversation('conversation-10')

  expect(chat.pinnedFiles.value).toEqual([{ path: 'wiki/gone.md', title: 'wiki/gone.md' }])
  expect(chat.messages.value[0]?.files).toEqual([{ path: 'wiki/gone.md', title: 'wiki/gone.md' }])
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
  expect(await chat.loadConversation('conversation-running')).toBe('loaded')
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

it.each(['wiki_chat_busy', 'database_unavailable', 'wiki_chat_connection_lost'])(
  'resends the question in a new conversation after %s rejected it before a turn was saved',
  async code => {
    const chat = mountChat()
    askWikiMock.mockRejectedValueOnce(new WikiChatRequestError(code))
    await chat.send('question')
    const assistant = chat.messages.value.at(-1)!
    expect(assistant.errorCode).toBe(code)

    answerWith('recovered')
    chat.retry(assistant.id)
    await flushPromises()
    expect(lastInput()).toEqual({ question: 'question', pinned_paths: [] })
    expect(chat.messages.value.at(-1)?.text).toBe('recovered')
  },
)

it('resends the question to the existing conversation instead of sending a local retry id', async () => {
  const chat = mountChat()
  answerWith('first')
  await chat.send('first')
  askWikiMock.mockRejectedValueOnce(new WikiChatRequestError('wiki_chat_busy'))
  await chat.send('second')

  answerWith('recovered')
  chat.retry(chat.messages.value.at(-1)!.id)
  await flushPromises()
  expect(lastInput()).toEqual({ conversation_id: 'conversation-1', question: 'second', pinned_paths: [] })
})

it('clears the previous attempt graph and storage flags when retrying a saved answer', async () => {
  const chat = mountChat()
  askWikiMock.mockImplementationOnce(async (input: WikiChatAskInput, handlers) => {
    handlers.onTurn({
      conversation_id: input.conversation_id ?? 'conversation-1',
      user_message_id: 'user-1', assistant_message_id: 'assistant-1',
    })
    handlers.onGraph(graphWith(1, 'failed attempt'))
    handlers.onDone({ citations: [], stop_reason: 'timeout', saved: false, content_truncated: true, graph_saved: true })
  })
  await chat.send('question')
  expect(chat.activeGraphMessageId.value).toBe('assistant-1')

  let release: () => void = () => undefined
  askWikiMock.mockImplementationOnce(() => new Promise<void>(resolve => { release = resolve }))
  chat.retry('assistant-1')
  const assistant = chat.messages.value.at(-1)!
  expect(assistant.graph).toBeNull()
  expect(assistant.saved).toBeUndefined()
  expect(assistant.contentTruncated).toBeUndefined()
  expect(chat.activeGraph.value).toBeNull()
  release()
  await flushPromises()
})

function savedAnswer(id: string, seq: number, graph: AnswerGraph | null): WikiConversationDetail['messages'][number] {
  return {
    id, seq, role: 'assistant', content: id, content_truncated: false, status: 'done', error_code: null,
    pinned_paths: [], tools: [], citations: [], graph, created_at_ms: 1, updated_at_ms: 1,
  }
}

it('opens on the latest answer whose graph has nodes', async () => {
  getWikiConversationMock.mockResolvedValue({
    id: 'conversation-graph', title: 'Graphs', created_at_ms: 1, updated_at_ms: 2,
    messages: [
      savedAnswer('with-evidence', 1, graphWith(1, 'real graph')),
      savedAnswer('no-evidence', 2, { revision: 1, nodes: [], edges: [], unresolved: [], truncated: false }),
    ],
  } satisfies WikiConversationDetail)
  const chat = mountChat()
  await chat.loadConversation('conversation-graph')
  expect(chat.activeGraphMessageId.value).toBe('with-evidence')
  expect(chat.activeGraph.value?.nodes[0]?.title).toBe('real graph')
})

it('reports a missing conversation as not found and forgets it', async () => {
  getWikiConversationMock.mockRejectedValue(new ApiError('gone', 404, 'wiki_conversation_not_found'))
  const chat = mountChat()
  expect(await chat.loadConversation('missing')).toBe('not_found')
  expect(chat.conversationId.value).toBeNull()
  expect(chat.conversationLoadFailed.value).toBe(false)
})

it.each([
  ['a database outage', new ApiError('down', 503, 'database_unavailable')],
  ['a network error', new TypeError('fetch failed')],
])('keeps the conversation id after %s and recovers when reloaded', async (_name, failure) => {
  getWikiConversationMock.mockRejectedValueOnce(failure)
  const chat = mountChat()
  expect(await chat.loadConversation('conversation-5')).toBe('failed')
  expect(chat.conversationId.value).toBe('conversation-5')
  expect(chat.conversationLoadFailed.value).toBe(true)
  expect(chat.readOnlyConversation.value).toBe(true)
  await chat.send('must not reach an unseen conversation')
  expect(askWikiMock).not.toHaveBeenCalled()

  getWikiConversationMock.mockResolvedValueOnce({
    id: 'conversation-5', title: 'Back', created_at_ms: 1, updated_at_ms: 2,
    messages: [savedAnswer('answer-5', 1, null)],
  } satisfies WikiConversationDetail)
  chat.refreshConversation()
  await flushPromises()
  expect(chat.conversationLoadFailed.value).toBe(false)
  expect(chat.readOnlyConversation.value).toBe(false)
  expect(chat.messages.value[0]?.text).toBe('answer-5')
})

it('keeps the visible messages when reloading the same conversation fails', async () => {
  getWikiConversationMock.mockResolvedValueOnce({
    id: 'conversation-6', title: 'Kept', created_at_ms: 1, updated_at_ms: 2,
    messages: [savedAnswer('answer-6', 1, null)],
  } satisfies WikiConversationDetail)
  const chat = mountChat()
  await chat.loadConversation('conversation-6')
  getWikiConversationMock.mockRejectedValueOnce(new ApiError('down', 503, 'database_unavailable'))
  chat.refreshConversation()
  await flushPromises()
  expect(chat.messages.value[0]?.text).toBe('answer-6')
  expect(chat.conversationLoadFailed.value).toBe(true)
})

import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, it, vi } from 'vitest'
import { defineComponent } from 'vue'
import type { AnswerGraph, WikiChatInputMessage } from '@/api/wiki-chat'
import { useWikiChat } from './useWikiChat'

const { askWikiMock } = vi.hoisted(() => ({ askWikiMock: vi.fn() }))
vi.mock('@/api/wiki-chat', async importOriginal => ({
  ...await importOriginal<typeof import('@/api/wiki-chat')>(),
  askWiki: askWikiMock,
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

function answerWith(text: string, graphs: AnswerGraph[] = []) {
  askWikiMock.mockImplementationOnce(async (_history, handlers) => {
    graphs.forEach(graph => handlers.onGraph(graph))
    handlers.onDelta(text)
    handlers.onDone({ citations: [], stop_reason: 'finished' })
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

function lastHistory(): WikiChatInputMessage[] {
  return askWikiMock.mock.lastCall![0]
}

beforeEach(() => {
  askWikiMock.mockReset()
})

it('drops the oldest messages once the history would pass 16 KB', async () => {
  const chat = mountChat()
  for (const turn of ['first', 'second', 'third']) {
    answerWith(`${turn} ${'é'.repeat(3000)}`)
    await chat.send(turn)
  }
  answerWith('done')
  await chat.send('fourth')

  const history = lastHistory()
  expect(new TextEncoder().encode(history.map(message => message.content).join('')).length).toBeLessThanOrEqual(16 * 1024)
  expect(history[0]?.role).toBe('user')
  expect(history[0]?.content).toBe('second')
  expect(history.at(-1)?.content).toBe('fourth')
})

it('sends at most 12 messages and still sends an oversized question', async () => {
  const chat = mountChat()
  for (let turn = 0; turn < 8; turn += 1) {
    answerWith(`answer ${turn}`)
    await chat.send(`question ${turn}`)
  }
  answerWith('done')
  await chat.send('latest')
  expect(lastHistory()).toHaveLength(11)
  expect(lastHistory()[0]?.role).toBe('user')

  const oversized = 'x'.repeat(17 * 1024)
  answerWith('done')
  await chat.send(oversized)
  expect(lastHistory()).toEqual([{ role: 'user', content: oversized }])
})

it('keeps one graph object per revision in a turn but accepts the same revision in a new answer', async () => {
  const chat = mountChat()
  answerWith('first', [graphWith(1, 'first answer'), graphWith(1, 'resent')])
  await chat.send('one')
  expect(chat.activeGraph.value?.nodes[0]?.title).toBe('first answer')

  answerWith('second', [graphWith(1, 'second answer')])
  await chat.send('two')
  expect(chat.activeGraph.value?.nodes[0]?.title).toBe('second answer')

  chat.showGraphOf(chat.messages.value[1]!.id)
  expect(chat.activeGraph.value?.nodes[0]?.title).toBe('first answer')
})

it('sends the attached files with every turn until they are removed', async () => {
  const chat = mountChat()
  chat.pinFile({ path: 'wiki/a.md', title: 'A' })
  chat.pinFile({ path: 'wiki/a.md', title: 'A again' })
  chat.pinFile({ path: 'sources/b.md', title: 'B' })
  answerWith('first')
  await chat.send('one')
  expect(askWikiMock.mock.lastCall![3]).toEqual(['wiki/a.md', 'sources/b.md'])
  expect(chat.messages.value[0]?.files?.map(file => file.title)).toEqual(['A', 'B'])

  chat.unpinFile('wiki/a.md')
  answerWith('second')
  await chat.send('two')
  expect(askWikiMock.mock.lastCall![3]).toEqual(['sources/b.md'])
  expect(chat.messages.value[0]?.files).toHaveLength(2)
})

it('stops at the attachment limit and retries a turn with the files it was asked with', async () => {
  const chat = mountChat()
  for (const name of ['a', 'b', 'c', 'd', 'e']) chat.pinFile({ path: `wiki/${name}.md`, title: name })
  expect(chat.pinnedFiles.value.map(file => file.title)).toEqual(['a', 'b', 'c', 'd'])

  askWikiMock.mockImplementationOnce(async (_history, handlers) => handlers.onError('provider_failed'))
  await chat.send('question')
  chat.unpinFile('wiki/a.md')
  answerWith('recovered')
  chat.retry(chat.messages.value[1]!.id)
  await flushPromises()
  expect(askWikiMock.mock.lastCall![3]).toEqual(['wiki/a.md', 'wiki/b.md', 'wiki/c.md', 'wiki/d.md'])

  chat.reset()
  expect(chat.pinnedFiles.value).toEqual([])
})

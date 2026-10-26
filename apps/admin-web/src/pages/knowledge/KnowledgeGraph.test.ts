import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, it, vi } from 'vitest'
import { ApiError } from '@/api/errors'
import { setLocale, translate } from '@/composables/useI18n'
import KnowledgeGraph from './KnowledgeGraph.vue'

const { requestJsonMock, events, drawMock, fitMock } = vi.hoisted(() => ({
  requestJsonMock: vi.fn(),
  events: {} as Record<string, (event: { target: { id: () => string } }) => void>,
  drawMock: vi.fn(),
  fitMock: vi.fn(),
}))
vi.mock('@/api/client', () => ({
  requestJson: requestJsonMock,
  withQuery: (path: string, query: Record<string, string | number>) =>
    path + '?' + new URLSearchParams(
      Object.entries(query).map(([name, value]) => [name, String(value)]),
    ).toString(),
}))
vi.mock('cytoscape', () => ({
  default: (options: unknown) => {
    drawMock(options)
    return {
      destroy: vi.fn(),
      on: (event: string, selector: string, callback: (event: { target: { id: () => string } }) => void) => {
        events[event + ':' + selector] = callback
      },
      fit: fitMock,
      layout: () => ({ run: vi.fn() }),
      getElementById: () => ({ select: vi.fn() }),
    }
  },
}))

const snapshot = {
  nodes: [
    { id: 'wiki_agent', path: 'wiki/AI/Agent.md', title: 'Agent', status: 'verified', revision: 2 },
    { id: 'wiki_llm', path: 'wiki/AI/LLM.md', title: 'LLM', status: 'verified', revision: 1 },
  ],
  edges: [{
    source_id: 'wiki_agent', target_id: 'wiki_llm', kind: 'wikilink', lines: [8],
    references: [{ line: 8, raw_target: 'AI/LLM#Inference|Language Model',
      alias: 'Language Model', anchor: 'Inference' }],
  }],
  unresolved: [],
  truncated: false,
}

beforeEach(() => {
  vi.clearAllMocks()
  for (const key of Object.keys(events)) delete events[key]
  setLocale('en')
  requestJsonMock.mockResolvedValue(snapshot)
})

it('selects an edge and exposes the original Wikilink alias and anchor', async () => {
  const wrapper = mount(KnowledgeGraph, { props: { centerId: 'wiki_agent', reloadToken: 0 } })
  try {
    await flushPromises()
    expect(drawMock).toHaveBeenCalledOnce()
    expect(wrapper.text()).toContain('Outgoing (1)')
    expect(wrapper.text()).toContain('Backlinks (0)')
    expect(events['tap:edge']).toBeTypeOf('function')
    events['tap:edge']({ target: { id: () => 'link_0' } })
    await flushPromises()
    expect(wrapper.text()).toContain('Language Model')
    expect(wrapper.text()).toContain('Inference')
    expect(wrapper.text()).toContain('Line 8')
    const open = wrapper.findAll('button').find(button =>
      button.text() === translate('knowledgeV3.graphOpenSource'))
    expect(open?.exists()).toBe(true)
    await open!.trigger('click')
    expect(wrapper.emitted('open')?.[0]).toEqual(['wiki_agent'])
  } finally { wrapper.unmount() }
})

it('queries the selected one-hop local neighborhood and can re-layout', async () => {
  const wrapper = mount(KnowledgeGraph, { props: { centerId: 'wiki_agent', reloadToken: 0 } })
  try {
    await flushPromises()
    const local = wrapper.findAll('button').find(button =>
      button.text() === translate('knowledgeV3.graphLocal'))!
    await local.trigger('click')
    await flushPromises()
    expect(requestJsonMock).toHaveBeenCalledWith(expect.stringContaining('scope=local'))
    expect(requestJsonMock).toHaveBeenCalledWith(expect.stringContaining('center_id=wiki_agent'))
    const relayout = wrapper.findAll('button').find(button =>
      button.text() === translate('knowledgeV3.graphRelayout'))
    await relayout!.trigger('click')
    expect(fitMock).not.toHaveBeenCalled()
  } finally { wrapper.unmount() }
})

it('offers an index rebuild when Graph is stale', async () => {
  requestJsonMock.mockRejectedValue(new ApiError('index_stale', 503, 'index_stale'))
  const wrapper = mount(KnowledgeGraph, { props: { centerId: 'wiki_agent', reloadToken: 0 } })
  try {
    await flushPromises()
    const rebuild = wrapper.findAll('button').find(button =>
      button.text() === translate('knowledgeV3.graphRebuild'))
    expect(rebuild?.exists()).toBe(true)
    await rebuild!.trigger('click')
    expect(wrapper.emitted('rebuild')).toHaveLength(1)
  } finally { wrapper.unmount() }
})

it('keeps non-zero flex sizing when Cytoscape applies position:relative and drops dangling or duplicate edges', async () => {
  requestJsonMock.mockResolvedValue({
    nodes: [
      { id: 'wiki_agent', path: 'wiki/AI/Agent.md', title: 'Agent', status: 'verified', revision: 2 },
      { id: 'wiki_llm', path: 'wiki/AI/LLM.md', title: 'LLM', status: 'verified', revision: 1 },
    ],
    edges: [
      {
        source_id: 'wiki_agent',
        target_id: 'sources/missing-node.md',
        kind: 'wikilink',
        lines: [3],
        references: [{ line: 3, raw_target: 'sources/missing-node.md' }],
      },
      {
        source_id: 'wiki_agent',
        target_id: 'wiki_llm',
        kind: 'wikilink',
        lines: [8],
        references: [{ line: 8, raw_target: 'AI/LLM', alias: 'LLM 1' }],
      },
      {
        source_id: 'wiki_agent',
        target_id: 'wiki_llm',
        kind: 'wikilink',
        lines: [14],
        references: [{ line: 14, raw_target: 'AI/LLM', alias: 'LLM 2' }],
      },
    ],
    unresolved: [],
    truncated: false,
  })
  const wrapper = mount(KnowledgeGraph, { props: { centerId: 'wiki_agent', reloadToken: 0 } })
  try {
    await flushPromises()
    const canvas = wrapper.find('[role="img"]')
    expect(canvas.classes()).not.toContain('absolute')
    expect(canvas.classes()).toContain('flex-1')
    expect(canvas.classes()).toContain('min-h-[410px]')

    expect(drawMock).toHaveBeenCalledOnce()
    const options = drawMock.mock.calls[0]?.[0] as {
      elements: Array<{ data: { id: string; source?: string; target?: string } }>
    }
    const edgeElements = options.elements.filter(el => 'source' in el.data)
    expect(edgeElements).toEqual([
      {
        data: {
          id: 'link_0',
          source: 'wiki_agent',
          target: 'wiki_llm',
          kind: 'wikilink',
        },
      },
    ])
    expect(wrapper.text()).toContain('Outgoing (1)')
    expect(wrapper.text()).toContain('→ LLM · 8, 14')
  } finally {
    wrapper.unmount()
  }
})

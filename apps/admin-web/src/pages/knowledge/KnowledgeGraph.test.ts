import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, it, vi } from 'vitest'
import { ApiError } from '@/api/errors'
import { setLocale, translate } from '@/composables/useI18n'
import KnowledgeGraph from './KnowledgeGraph.vue'
import { KNOWLEDGE_BACKEND_KEY, MemoryKnowledgeBackend, type KnowledgeGraph as KnowledgeGraphData } from './knowledgeBackend'

const { events, drawMock, fitMock, centerMock } = vi.hoisted(() => ({
  events: {} as Record<string, (event: { target: { id: () => string }; originalEvent?: { metaKey?: boolean } }) => void>,
  drawMock: vi.fn(),
  fitMock: vi.fn(),
  centerMock: vi.fn(),
}))
vi.mock('cytoscape', () => ({
  default: (options: unknown) => {
    drawMock(options)
    return {
      destroy: vi.fn(),
      on: (event: string, selector: string, callback: (event: { target: { id: () => string }; originalEvent?: { metaKey?: boolean } }) => void) => {
        events[event + ':' + selector] = callback
      },
      fit: fitMock,
      center: centerMock,
      layout: () => ({ run: vi.fn() }),
      getElementById: () => ({ select: vi.fn() }),
    }
  },
}))

const snapshot = {
  nodes: [
    { id: 'wiki/AI/Agent.md', path: 'wiki/AI/Agent.md', title: 'Agent', status: 'verified', revision: 2 },
    { id: 'wiki/AI/LLM.md', path: 'wiki/AI/LLM.md', title: 'LLM', status: 'verified', revision: 1 },
  ],
  edges: [{
    source_id: 'wiki/AI/Agent.md', target_id: 'wiki/AI/LLM.md', kind: 'wikilink', lines: [8],
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
})

function mountGraph(
  props: InstanceType<typeof KnowledgeGraph>['$props'] = { reloadToken: 0 },
  graph: KnowledgeGraphData = snapshot,
  graphError?: unknown,
) {
  const backend = new MemoryKnowledgeBackend({ graph, graphError })
  const wrapper = mount(KnowledgeGraph, {
    props,
    global: { provide: { [KNOWLEDGE_BACKEND_KEY as symbol]: backend } },
  })
  return { wrapper, backend }
}

it('selects an edge and exposes the original Wikilink alias and anchor', async () => {
  const { wrapper } = mountGraph({ centerId: 'wiki/AI/Agent.md', reloadToken: 0 })
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
    expect(wrapper.emitted('open')?.[0]).toEqual(['wiki/AI/Agent.md'])
  } finally { wrapper.unmount() }
})

it('opens a node beside Graph on Ctrl or Cmd click', async () => {
  const { wrapper } = mountGraph({ reloadToken: 0 })
  try {
    await flushPromises()
    events['tap:node']({ target: { id: () => 'wiki/AI/Agent.md' }, originalEvent: { metaKey: true } })
    expect(wrapper.emitted('open-side')?.[0]).toEqual(['wiki/AI/Agent.md'])
    expect(wrapper.emitted('open')).toBeUndefined()
    events['tap:node']({ target: { id: () => 'wiki/AI/Agent.md' }, originalEvent: {} })
    expect(wrapper.emitted('open')?.[0]).toEqual(['wiki/AI/Agent.md'])
  } finally { wrapper.unmount() }
})

it('queries the selected one-hop local neighborhood and can re-layout', async () => {
  const { wrapper, backend } = mountGraph({ centerId: 'wiki/AI/Agent.md', reloadToken: 0 })
  const graph = vi.spyOn(backend, 'graph')
  try {
    await flushPromises()
    const local = wrapper.findAll('button').find(button =>
      button.text() === translate('knowledgeV3.graphLocal'))!
    await local.trigger('click')
    await flushPromises()
    expect(graph).toHaveBeenLastCalledWith({ scope: 'wiki', relative: 'AI/Agent.md' })
    const relayout = wrapper.findAll('button').find(button =>
      button.text() === translate('knowledgeV3.graphRelayout'))
    await relayout!.trigger('click')
    expect(fitMock).not.toHaveBeenCalled()
  } finally { wrapper.unmount() }
})

it('queries a topic and styles retrieved tiers without changing the node type colors', async () => {
  const querySnapshot = {
      ...snapshot,
      nodes: snapshot.nodes.map((node, index) => ({
        ...node,
        tier: index === 0 ? 'retrieved' : 'context',
        score: index === 0 ? 0.82 : null,
      })),
      truncated: true,
    }
  const { wrapper, backend } = mountGraph({ centerId: 'wiki/AI/Agent.md', reloadToken: 0 }, querySnapshot)
  const graph = vi.spyOn(backend.admin!, 'graph')
  try {
    await flushPromises()
    const query = wrapper.findAll('button').find(button =>
      button.text() === translate('knowledgeV3.graphQuery'))!
    await query.trigger('click')
    const input = wrapper.get('input[aria-label="' + translate('knowledgeV3.graphQueryInput') + '"]')
    await input.setValue('trading')
    await wrapper.find('form.kg-query-form').trigger('submit')
    await flushPromises()

    expect(graph).toHaveBeenLastCalledWith({ scope: 'query', limit: 500, query: 'trading' })
    expect(wrapper.text()).toContain(translate('knowledgeV3.graphTruncated'))
    const options = drawMock.mock.calls.at(-1)?.[0] as {
      elements: Array<{ data: { tier?: string } }>
      style: Array<{ selector: string; style: Record<string, unknown> }>
    }
    expect(options.elements[0]?.data.tier).toBe('retrieved')
    expect(options.style.map(rule => rule.selector)).toContain('node[tier = "retrieved"]')
    expect(options.style.map(rule => rule.selector)).toContain('node[group = "concept"]')
    expect(wrapper.text()).toContain(translate('knowledgeV3.graphTierLegend'))
  } finally { wrapper.unmount() }
})

it('draws an answer graph from its prop, avoids API reloads, and opens node details in place', async () => {
  const answerGraph = {
    revision: 1,
    nodes: [{ ...snapshot.nodes[0]!, tier: 'read' as const, score: 0.77 }],
    edges: [],
    unresolved: [],
    truncated: false,
  }
  const { wrapper, backend } = mountGraph({ centerId: null, reloadToken: 0, answerGraph })
  try {
    await flushPromises()
    const graph = vi.spyOn(backend.admin!, 'graph')
    expect(drawMock).toHaveBeenCalledOnce()
    expect(wrapper.text()).toContain(translate('knowledgeV3.graphAnswer'))
    expect(drawMock.mock.calls[0]?.[0].elements[0]?.data.tier).toBe('read')

    events['tap:node']?.({ target: { id: () => 'wiki/AI/Agent.md' } })
    await flushPromises()
    expect(wrapper.get('aside.kg-inspect').classes()).not.toContain('is-hidden')
    expect(wrapper.emitted('open')).toBeUndefined()
    const openPage = wrapper.get('.kg-inspect-actions .kg-btn-primary')
    await openPage.trigger('click')
    expect(wrapper.emitted('open')?.[0]).toEqual(['wiki/AI/Agent.md'])

    const exposed = wrapper.vm as unknown as {
      focusNode: (id: string) => void
      highlightNode: (id: string | null) => void
    }
    exposed.focusNode('wiki/AI/Agent.md')
    exposed.highlightNode('wiki/AI/Agent.md')
    exposed.highlightNode(null)
    expect(centerMock).toHaveBeenCalledOnce()

    await wrapper.setProps({ answerGraph: { ...answerGraph, revision: 2 } })
    await flushPromises()
    expect(drawMock).toHaveBeenCalledTimes(2)
    const earlierAnswer = { ...answerGraph, revision: 2, nodes: [{ ...answerGraph.nodes[0]!, title: 'Earlier answer' }] }
    await wrapper.setProps({ answerGraph: earlierAnswer })
    await flushPromises()
    expect(drawMock).toHaveBeenCalledTimes(3)
    expect(drawMock.mock.calls[2]?.[0].elements[0]?.data.title).toBe('Earlier answer')
    await wrapper.setProps({ reloadToken: 1 })
    await flushPromises()
    expect(drawMock).toHaveBeenCalledTimes(3)
    expect(graph).not.toHaveBeenCalled()
  } finally { wrapper.unmount() }
})

it('leaves Global nodes without a tier so answer styling cannot change them', async () => {
  const { wrapper } = mountGraph({ centerId: null, reloadToken: 0 })
  try {
    await flushPromises()
    const elements = drawMock.mock.calls.at(-1)?.[0].elements as Array<{ data: { tier?: string } }>
    expect(elements[0]?.data.tier).toBeUndefined()
  } finally { wrapper.unmount() }
})

it('offers an index rebuild when Graph is stale', async () => {
  const { wrapper } = mountGraph({ centerId: 'wiki/AI/Agent.md', reloadToken: 0 }, snapshot,
    new ApiError('index_stale', 503, 'index_stale'))
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
  const danglingSnapshot = {
    nodes: [
      { id: 'wiki/AI/Agent.md', path: 'wiki/AI/Agent.md', title: 'Agent', status: 'verified', revision: 2 },
      { id: 'wiki/AI/LLM.md', path: 'wiki/AI/LLM.md', title: 'LLM', status: 'verified', revision: 1 },
    ],
    edges: [
      {
        source_id: 'wiki/AI/Agent.md',
        target_id: 'sources/missing-node.md',
        kind: 'wikilink',
        lines: [3],
        references: [{ line: 3, raw_target: 'sources/missing-node.md' }],
      },
      {
        source_id: 'wiki/AI/Agent.md',
        target_id: 'wiki/AI/LLM.md',
        kind: 'wikilink',
        lines: [8],
        references: [{ line: 8, raw_target: 'AI/LLM', alias: 'LLM 1' }],
      },
      {
        source_id: 'wiki/AI/Agent.md',
        target_id: 'wiki/AI/LLM.md',
        kind: 'wikilink',
        lines: [14],
        references: [{ line: 14, raw_target: 'AI/LLM', alias: 'LLM 2' }],
      },
    ],
    unresolved: [],
    truncated: false,
  }
  const { wrapper } = mountGraph({ centerId: 'wiki/AI/Agent.md', reloadToken: 0 }, danglingSnapshot)
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
          source: 'wiki/AI/Agent.md',
          target: 'wiki/AI/LLM.md',
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

it('opens the note directly without showing the graph node info card when a node is clicked, and supports increasing/decreasing font size', async () => {
  const { wrapper } = mountGraph({ centerId: 'wiki/AI/Agent.md', reloadToken: 0 })
  try {
    await flushPromises()
    const inspect = wrapper.get('aside.kg-inspect')
    expect(inspect.classes()).toContain('is-hidden')

    // Tapping a node opens the note directly instead of showing the node inspector card
    events['tap:node']?.({ target: { id: () => 'wiki/AI/Agent.md' } })
    await flushPromises()
    expect(inspect.classes()).toContain('is-hidden')
    expect(wrapper.emitted('open')?.[0]).toEqual(['wiki/AI/Agent.md'])

    // Increasing and decreasing font size updates the Cytoscape node style
    const initialOptions = drawMock.mock.calls.at(-1)?.[0] as {
      style: Array<{ selector: string; style: Record<string, unknown> }>
    }
    const initialFontSize = initialOptions.style.find(s => s.selector === 'node')?.style['font-size'] as number

    await wrapper.get('button[aria-label="Increase font size"]').trigger('click')
    await flushPromises()
    const increasedOptions = drawMock.mock.calls.at(-1)?.[0] as {
      style: Array<{ selector: string; style: Record<string, unknown> }>
    }
    const increasedFontSize = increasedOptions.style.find(s => s.selector === 'node')?.style['font-size'] as number
    expect(increasedFontSize).toBeGreaterThan(initialFontSize)

    await wrapper.get('button[aria-label="Decrease font size"]').trigger('click')
    await flushPromises()
    const restoredOptions = drawMock.mock.calls.at(-1)?.[0] as {
      style: Array<{ selector: string; style: Record<string, unknown> }>
    }
    const restoredFontSize = restoredOptions.style.find(s => s.selector === 'node')?.style['font-size'] as number
    expect(restoredFontSize).toBe(initialFontSize)
  } finally {
    wrapper.unmount()
  }
})

it('supports floating node search, display settings menu, and centers computed node positions around (0, 0)', async () => {
  const { wrapper } = mountGraph({ centerId: 'wiki/AI/Agent.md', reloadToken: 0 })
  try {
    await flushPromises()
    const options = drawMock.mock.calls[0]?.[0] as {
      layout: { positions: (node: { id: () => string }) => { x: number; y: number } }
    }
    const p1 = options.layout.positions({ id: () => 'wiki/AI/Agent.md' })
    const p2 = options.layout.positions({ id: () => 'wiki/AI/LLM.md' })
    // Bounding box center is centered at (0, 0)
    expect(Math.abs((p1.x + p2.x) / 2)).toBeLessThanOrEqual(1)
    expect(Math.abs((p1.y + p2.y) / 2)).toBeLessThanOrEqual(1)

    // Search palette opens, filters nodes, and clicking a result opens that note directly
    await wrapper.get('button[aria-label="Search nodes"]').trigger('click')
    expect(wrapper.get('.kg-search').classes()).toContain('open')
    const searchInput = wrapper.get('input[aria-label="Search a node"]')
    await searchInput.setValue('LLM')
    const results = wrapper.findAll('.kg-search-result')
    expect(results).toHaveLength(1)
    expect(results[0]!.text()).toContain('LLM')
    await results[0]!.trigger('click')
    await flushPromises()
    expect(wrapper.get('.kg-search').classes()).not.toContain('open')
    expect(wrapper.emitted('open')?.[0]).toEqual(['wiki/AI/LLM.md'])

    // Display settings menu toggles legend
    await wrapper.get('button[aria-label="Display settings"]').trigger('click')
    expect(wrapper.get('.kg-menu').classes()).toContain('open')
  } finally {
    wrapper.unmount()
  }
})


<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import cytoscape, { type Core } from 'cytoscape'
import { requestJson, withQuery } from '@/api/client'
import { formatApiError, isApiError } from '@/api/errors'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useTheme } from '@/composables/useTheme'

type GraphNode = {
  id: string
  path: string
  title: string
  status: string
  revision: number
  category?: string
  tags?: string[]
}
type GraphReference = { line: number; raw_target: string; alias?: string; anchor?: string }
type GraphEdge = { source_id: string; target_id: string; kind: string; lines: number[]; references: GraphReference[] }
type GraphUnresolved = { source_id: string; source_line: number; raw_target: string; reason: string }
type GraphSnapshot = { nodes: GraphNode[]; edges: GraphEdge[]; unresolved: GraphUnresolved[]; truncated: boolean }
type NodeGroup = 'concept' | 'entity' | 'synthesis' | 'source' | 'system' | 'draft'

const props = defineProps<{ centerId?: string | null; reloadToken: number }>()
const emit = defineEmits<{ open: [id: string]; rebuild: [] }>()
const { t } = useI18n()
const { dark } = useTheme()
const scope = ref<'global' | 'local'>('global')
const depth = ref(1)
const graph = ref<GraphSnapshot | null>(null)
const selectedId = ref<string | null>(null)
const loading = ref(false)
const error = ref('')
const isStale = ref(false)
const hoverText = ref('')
const selectedEdgeId = ref<string | null>(null)
const canvas = ref<HTMLElement | null>(null)
let cy: Core | null = null
let requestVersion = 0
let layoutRunCount = 0

function classifyNodeGroup(node: GraphNode): NodeGroup {
  if (node.status === 'draft') return 'draft'
  const cat = (node.category ?? '').toLowerCase()
  const path = (node.path || node.id).toLowerCase()
  if (cat === 'concept' || path.includes('/concepts/')) return 'concept'
  if (cat === 'entity' || path.includes('/entities/')) return 'entity'
  if (cat === 'synthesis' || path.includes('/syntheses/')) return 'synthesis'
  if (cat === 'system' || path === 'wiki/index.md' || path === 'wiki/log.md') return 'system'
  if (cat.includes('source') || path.startsWith('sources/') || path.includes('/sources/')) return 'source'
  return 'concept'
}

function compactNodeLabel(title: string, maxChars = 24): string {
  const trimmed = title.trim()
  if (trimmed.length <= maxChars) return trimmed
  const sliced = trimmed.slice(0, maxChars)
  const lastSpace = sliced.lastIndexOf(' ')
  return (lastSpace >= 14 ? sliced.slice(0, lastSpace) : sliced).replace(/[—–\-:,.\s]+$/, '') + '…'
}

const visibleGraph = computed<{ nodes: GraphNode[]; edges: GraphEdge[] }>(() => {
  const snapshot = graph.value
  if (!snapshot) return { nodes: [], edges: [] }
  const allNodeIds = new Set(snapshot.nodes.map(n => n.id))
  const merged = new Map<string, GraphEdge>()
  for (const edge of snapshot.edges) {
    // Exclude self-loops and edges referencing missing nodes so the graph stays clean like Obsidian.
    if (
      edge.source_id === edge.target_id ||
      !allNodeIds.has(edge.source_id) ||
      !allNodeIds.has(edge.target_id)
    ) {
      continue
    }
    const key = `${edge.source_id}\0${edge.target_id}\0${edge.kind}`
    const existing = merged.get(key)
    if (existing) {
      for (const line of edge.lines) {
        if (!existing.lines.includes(line)) existing.lines.push(line)
      }
      existing.references.push(...edge.references)
    } else {
      merged.set(key, {
        source_id: edge.source_id,
        target_id: edge.target_id,
        kind: edge.kind,
        lines: [...edge.lines],
        references: [...edge.references],
      })
    }
  }
  const deduplicatedEdges = [...merged.values()]
  if (scope.value !== 'local' || !props.centerId || !allNodeIds.has(props.centerId)) {
    return { nodes: snapshot.nodes, edges: deduplicatedEdges }
  }
  const reachable = new Set<string>([props.centerId])
  let frontier = new Set<string>([props.centerId])
  const maxHops = Math.max(1, depth.value)
  for (let hop = 0; hop < maxHops; hop++) {
    const next = new Set<string>()
    for (const edge of deduplicatedEdges) {
      if (frontier.has(edge.source_id) && !reachable.has(edge.target_id)) {
        reachable.add(edge.target_id)
        next.add(edge.target_id)
      }
      if (frontier.has(edge.target_id) && !reachable.has(edge.source_id)) {
        reachable.add(edge.source_id)
        next.add(edge.source_id)
      }
    }
    if (next.size === 0) break
    frontier = next
  }
  return {
    nodes: snapshot.nodes.filter(n => reachable.has(n.id)),
    edges: deduplicatedEdges.filter(e => reachable.has(e.source_id) && reachable.has(e.target_id)),
  }
})

const visibleNodes = computed(() => visibleGraph.value.nodes)
const visibleEdges = computed(() => visibleGraph.value.edges)
const current = computed(() => visibleNodes.value.find(node => node.id === selectedId.value) ?? null)
const selectedEdge = computed(() => {
  if (!selectedEdgeId.value) return null
  const index = Number(selectedEdgeId.value.replace('link_', ''))
  return visibleEdges.value[index] ?? null
})
const outgoing = computed(() => visibleEdges.value.filter(e => e.source_id === selectedId.value))
const incoming = computed(() => visibleEdges.value.filter(e => e.target_id === selectedId.value))
const unresolved = computed(() => (graph.value?.unresolved ?? []).filter(e => e.source_id === selectedId.value))
const names = computed(() => new Map((graph.value?.nodes ?? []).map(n => [n.id, n.title])))
function name(id: string): string { return names.value.get(id) ?? id }

function clampViewportZoom() {
  if (!cy || typeof cy.zoom !== 'function' || typeof cy.center !== 'function') return
  if (cy.zoom() > 1.15) {
    cy.zoom(1.15)
    cy.center()
  }
}

function computeObsidianPositions(
  nodes: GraphNode[],
  edges: GraphEdge[],
  seedOffset = 0,
): Record<string, { x: number; y: number }> {
  const n = nodes.length
  if (n === 0) return {}
  if (n === 1) return { [nodes[0]!.id]: { x: 0, y: 0 } }

  const containerW = Math.max(320, canvas.value?.clientWidth || 440)
  const containerH = Math.max(380, canvas.value?.clientHeight || 620)
  const aspectY = Math.max(1.0, Math.min(1.55, containerH / containerW))

  const idxById = new Map(nodes.map((node, i) => [node.id, i]))
  const degree = new Array<number>(n).fill(0)
  const adj: number[][] = Array.from({ length: n }, () => [])
  const seenPairs = new Set<string>()
  for (const e of edges) {
    const u = idxById.get(e.source_id)
    const v = idxById.get(e.target_id)
    if (u === undefined || v === undefined || u === v) continue
    const pairKey = u < v ? `${u}:${v}` : `${v}:${u}`
    if (seenPairs.has(pairKey)) continue
    seenPairs.add(pairKey)
    degree[u]!++
    degree[v]!++
    adj[u]!.push(v)
    adj[v]!.push(u)
  }

  const x = new Float64Array(n)
  const y = new Float64Array(n)
  const pinned = new Uint8Array(n)
  const focalId = (scope.value === 'local' ? props.centerId : null) ?? selectedId.value
  const centerIdx = focalId ? idxById.get(focalId) : undefined

  if (scope.value === 'local' && centerIdx !== undefined) {
    pinned[centerIdx] = 1
    x[centerIdx] = 0
    y[centerIdx] = 0
    const hop = new Int32Array(n).fill(2)
    hop[centerIdx] = 0
    for (const nb of adj[centerIdx]!) hop[nb] = 1
    const ring1: number[] = []
    const ring2: number[] = []
    for (let i = 0; i < n; i++) {
      if (i === centerIdx) continue
      if (hop[i] === 1) ring1.push(i)
      else ring2.push(i)
    }
    const baseAngle = -Math.PI / 2 + seedOffset * 0.45
    const r1 = Math.max(135, ring1.length * 28)
    ring1.forEach((idx, k) => {
      const angle = baseAngle + (2 * Math.PI * k) / Math.max(1, ring1.length)
      x[idx] = Math.cos(angle) * r1
      y[idx] = Math.sin(angle) * r1 * Math.min(1.25, aspectY)
    })
    const r2 = r1 + Math.max(125, ring2.length * 24)
    ring2.forEach((idx, k) => {
      const parents = adj[idx]!.filter(nb => hop[nb] === 1)
      if (parents.length > 0) {
        const px = parents.reduce((acc, p) => acc + x[p]!, 0) / parents.length
        const py = parents.reduce((acc, p) => acc + y[p]!, 0) / parents.length
        const pAngle = Math.atan2(py, px) + ((k % 3) - 1) * 0.22
        x[idx] = Math.cos(pAngle) * r2
        y[idx] = Math.sin(pAngle) * r2 * Math.min(1.25, aspectY)
      } else {
        const angle = baseAngle + (2 * Math.PI * (k + 0.5)) / Math.max(1, ring2.length)
        x[idx] = Math.cos(angle) * r2
        y[idx] = Math.sin(angle) * r2 * Math.min(1.25, aspectY)
      }
    })
  } else {
    const groupKey = (node: GraphNode) => {
      const parts = (node.path || node.id).replace(/^(wiki|sources)\//, '').split('/')
      return parts.slice(0, 2).join('/')
    }
    const groups = [...new Set(nodes.map(groupKey))].sort()
    const groupIdx = new Map(groups.map((g, i) => [g, i]))
    const maxDeg = Math.max(1, ...degree)
    const ordered = nodes
      .map((node, i) => ({ i, g: groupIdx.get(groupKey(node)) ?? 0, deg: degree[i]! }))
      .sort((a, b) => a.g - b.g || b.deg - a.deg)

    ordered.forEach((item, rank) => {
      if (item.deg === maxDeg && rank === 0) {
        x[item.i] = 0
        y[item.i] = 0
        return
      }
      const sectorAngle =
        seedOffset * 0.5 + (2 * Math.PI * item.g) / Math.max(1, groups.length)
      const jitter = ((rank % 5) - 2) * 0.2
      const angle = sectorAngle + jitter
      const radius = 95 + (rank % 4) * 58
      x[item.i] = Math.cos(angle) * radius
      y[item.i] = Math.sin(angle) * radius * aspectY
    })

    const ticks = 150
    for (let step = 0; step < ticks; step++) {
      const alpha = 1 - step / ticks
      const fx = new Float64Array(n)
      const fy = new Float64Array(n)

      for (let i = 0; i < n; i++) {
        for (let j = i + 1; j < n; j++) {
          let dx = x[j]! - x[i]!
          let dy = (y[j]! - y[i]!) / aspectY
          let d2 = dx * dx + dy * dy
          if (d2 < 1) {
            dx = (i - j) * 0.5
            dy = 0.5
            d2 = dx * dx + dy * dy
          }
          const d = Math.sqrt(d2)
          const repulse = (9500 * alpha) / d2
          const ux = (dx / d) * repulse
          const uy = (dy / d) * repulse * aspectY
          fx[i]! -= ux
          fy[i]! -= uy
          fx[j]! += ux
          fy[j]! += uy
        }
      }

      for (const e of edges) {
        const u = idxById.get(e.source_id)
        const v = idxById.get(e.target_id)
        if (u === undefined || v === undefined || u === v) continue
        const dx = x[v]! - x[u]!
        const dy = y[v]! - y[u]!
        const d = Math.hypot(dx, dy) || 1
        const weight = 1 / Math.sqrt(Math.max(1, degree[u]!) * Math.max(1, degree[v]!))
        const spring = (d - 115) * 0.18 * weight * alpha
        const ux = (dx / d) * spring
        const uy = (dy / d) * spring
        fx[u]! += ux
        fy[u]! += uy
        fx[v]! -= ux
        fy[v]! -= uy
      }

      for (let i = 0; i < n; i++) {
        fx[i]! -= x[i]! * 0.022 * alpha
        fy[i]! -= (y[i]! / aspectY) * 0.022 * alpha
        x[i]! += Math.max(-28, Math.min(28, fx[i]!))
        y[i]! += Math.max(-28, Math.min(28, fy[i]!))
      }
    }
  }

  // Anisotropic label-box collision resolution tailored to viewport aspect ratio.
  const minX = scope.value === 'local' ? 118 : 98
  const minY = scope.value === 'local' ? 66 : 62
  for (let pass = 0; pass < 55; pass++) {
    let moved = false
    for (let i = 0; i < n; i++) {
      for (let j = i + 1; j < n; j++) {
        const dx = x[j]! - x[i]!
        const dy = y[j]! - y[i]!
        const nx = dx / minX
        const ny = dy / minY
        const dist = Math.hypot(nx, ny)
        if (dist < 1) {
          moved = true
          const overlap = (1 - dist) * 0.55
          const ux = dist > 1e-4 ? (nx / dist) * minX * overlap : minX * 0.5
          const uy = dist > 1e-4 ? (ny / dist) * minY * overlap : minY * 0.5
          if (!pinned[i] && !pinned[j]) {
            x[i]! -= ux
            y[i]! -= uy
            x[j]! += ux
            y[j]! += uy
          } else if (pinned[i]) {
            x[j]! += ux * 2
            y[j]! += uy * 2
          } else {
            x[i]! -= ux * 2
            y[i]! -= uy * 2
          }
        }
      }
    }
    if (!moved) break
  }

  const out: Record<string, { x: number; y: number }> = {}
  for (let i = 0; i < n; i++) {
    out[nodes[i]!.id] = { x: Math.round(x[i]!), y: Math.round(y[i]!) }
  }
  return out
}

function buildLayoutOptions(nodes: GraphNode[], edges: GraphEdge[], seedOffset = 0) {
  const positions = computeObsidianPositions(nodes, edges, seedOffset)
  return {
    name: 'preset' as const,
    animate: false,
    fit: true,
    padding: 34,
    positions: (node: { id: () => string }) => positions[node.id()] ?? { x: 0, y: 0 },
  }
}

function fitGraph() {
  cy?.fit(undefined, 34)
  clampViewportZoom()
}
function relayout() {
  layoutRunCount++
  cy?.layout(buildLayoutOptions(visibleNodes.value, visibleEdges.value, layoutRunCount)).run()
  clampViewportZoom()
}
function focusEdge(edge: GraphEdge) {
  const index = visibleEdges.value.indexOf(edge)
  if (index < 0) return
  selectedEdgeId.value = 'link_' + index
  cy?.getElementById(selectedEdgeId.value).select()
}
function focusNode(id: string) {
  selectedEdgeId.value = null
  selectedId.value = id
  cy?.getElementById(id).select()
  applyFocusClasses(null)
}

function applyFocusClasses(hoveredId: string | null) {
  if (!cy || typeof cy.elements !== 'function') return
  const all = cy.elements()
  all.removeClass('is-dimmed is-highlighted is-active-edge')
  if (hoveredId) {
    const target = cy.getElementById(hoveredId)
    if (target && typeof target.closedNeighborhood === 'function') {
      const neighborhood = target.closedNeighborhood()
      all.not(neighborhood).addClass('is-dimmed')
      neighborhood.addClass('is-highlighted')
      return
    }
  }
  if (selectedId.value) {
    const selectedEl = cy.getElementById(selectedId.value)
    if (selectedEl && typeof selectedEl.connectedEdges === 'function') {
      selectedEl.connectedEdges().addClass('is-active-edge')
    }
  }
}

function draw() {
  cy?.destroy()
  cy = null
  if (!canvas.value || !graph.value) return
  const nodes = visibleNodes.value
  const edges = visibleEdges.value

  // Compute node degree to scale node size like Obsidian Graph View.
  const degreeById = new Map<string, number>()
  for (const n of nodes) degreeById.set(n.id, 0)
  for (const e of edges) {
    degreeById.set(e.source_id, (degreeById.get(e.source_id) ?? 0) + 1)
    degreeById.set(e.target_id, (degreeById.get(e.target_id) ?? 0) + 1)
  }

  // Obsidian Graph View semantic palette for dark & light themes.
  const palette = dark.value
    ? {
        canvasBg: '#11131a',
        text: '#dcdfe8',
        edge: '#565c75',
        edgeActive: '#a882ff',
        selected: '#3dd68c',
        selectedRing: '#3dd68c',
        concept: '#a882ff',
        entity: '#5ccfe6',
        synthesis: '#ffcc66',
        source: '#ff7a90',
        system: '#94a3b8',
        draft: '#f59e0b',
      }
    : {
        canvasBg: '#f8f9fc',
        text: '#232634',
        edge: '#9ca3bd',
        edgeActive: '#7c3aed',
        selected: '#10b981',
        selectedRing: '#10b981',
        concept: '#7c3aed',
        entity: '#0284c7',
        synthesis: '#d97706',
        source: '#e11d48',
        system: '#64748b',
        draft: '#d97706',
      }

  const isLocal = scope.value === 'local'
  const baseFontSize = isLocal ? 11.5 : 13.5

  const elements = [
    ...nodes.map(n => {
      const deg = degreeById.get(n.id) ?? 0
      const size = Math.min(22, (isLocal ? 12 : 11) + Math.round(Math.sqrt(deg) * 2.8))
      const group = classifyNodeGroup(n)
      return {
        data: {
          id: n.id,
          title: n.title,
          shortLabel: compactNodeLabel(n.title, isLocal ? 26 : 22),
          selectedLabel: compactNodeLabel(n.title, 38),
          status: n.status,
          group,
          size,
        },
      }
    }),
    ...edges.map((e, i) => ({
      data: { id: 'link_' + i, source: e.source_id, target: e.target_id, kind: e.kind },
    })),
  ]

  cy = cytoscape({
    container: canvas.value,
    elements,
    layout: buildLayoutOptions(nodes, edges, layoutRunCount),
    minZoom: 0.25,
    maxZoom: 2.2,
    wheelSensitivity: 0.24,
    style: [
      {
        selector: 'node',
        style: {
          'label': 'data(shortLabel)',
          'font-size': baseFontSize,
          'font-weight': 500,
          'text-wrap': 'wrap',
          'text-max-width': isLocal ? '130px' : '125px',
          'text-valign': 'bottom',
          'text-margin-y': 6,
          'color': palette.text,
          'text-outline-width': 3,
          'text-outline-color': palette.canvasBg,
          'text-outline-opacity': 0.94,
          'background-color': palette.concept,
          'width': 'data(size)',
          'height': 'data(size)',
          'border-width': 1.5,
          'border-color': palette.canvasBg,
          'border-opacity': 0.9,
          'transition-property': 'opacity, width, height, border-width',
          'transition-duration': 120,
        },
      },
      { selector: 'node[group = "concept"]', style: { 'background-color': palette.concept } },
      { selector: 'node[group = "entity"]', style: { 'background-color': palette.entity } },
      { selector: 'node[group = "synthesis"]', style: { 'background-color': palette.synthesis } },
      { selector: 'node[group = "source"]', style: { 'background-color': palette.source } },
      { selector: 'node[group = "system"]', style: { 'background-color': palette.system } },
      { selector: 'node[status = "draft"]', style: { 'background-color': palette.draft } },
      {
        selector: 'node:selected',
        style: {
          'label': 'data(selectedLabel)',
          'text-max-width': '165px',
          'background-color': palette.selected,
          'border-width': 3.5,
          'border-color': palette.selectedRing,
          'border-opacity': 0.45,
          'font-weight': 600,
          'z-index': 20,
        },
      },
      {
        selector: 'edge',
        style: {
          'width': isLocal ? 1.2 : 0.95,
          'line-color': palette.edge,
          'target-arrow-color': palette.edge,
          'target-arrow-shape': isLocal ? 'vee' : 'none',
          'arrow-scale': 0.65,
          'curve-style': 'straight',
          'opacity': isLocal ? 0.36 : 0.2,
          'transition-property': 'opacity, width, line-color, target-arrow-color',
          'transition-duration': 120,
        },
      },
      {
        selector: 'edge.is-active-edge',
        style: {
          'width': 1.65,
          'line-color': palette.edgeActive,
          'target-arrow-color': palette.edgeActive,
          'target-arrow-shape': 'vee',
          'arrow-scale': 0.7,
          'opacity': 0.78,
          'z-index': 12,
        },
      },
      {
        selector: 'edge:selected',
        style: {
          'width': 2.1,
          'line-color': palette.selected,
          'target-arrow-color': palette.selected,
          'target-arrow-shape': 'vee',
          'opacity': 0.95,
          'z-index': 15,
        },
      },
      {
        selector: '.is-dimmed',
        style: {
          'opacity': 0.13,
        },
      },
      {
        selector: 'node.is-highlighted',
        style: {
          'opacity': 1,
          'z-index': 18,
        },
      },
      {
        selector: 'edge.is-highlighted',
        style: {
          'opacity': 0.88,
          'width': 1.75,
          'line-color': palette.edgeActive,
          'target-arrow-color': palette.edgeActive,
          'target-arrow-shape': 'vee',
          'z-index': 14,
        },
      },
    ],
  })
  clampViewportZoom()
  cy.on('tap', 'node', event => {
    selectedEdgeId.value = null
    selectedId.value = event.target.id()
    applyFocusClasses(null)
  })
  cy.on('tap', 'edge', event => { selectedEdgeId.value = event.target.id() })
  cy.on('mouseover', 'node', event => {
    const id = event.target.id()
    const node = nodes.find(n => n.id === id)
    hoverText.value = node ? node.title + ' · ' + node.path : ''
    applyFocusClasses(id)
  })
  cy.on('mouseover', 'edge', event => {
    const edge = edges[Number(event.target.id().replace('link_', ''))]
    hoverText.value = edge ? name(edge.source_id) + ' → ' + name(edge.target_id) : ''
  })
  cy.on('mouseout', 'node, edge', () => {
    hoverText.value = ''
    applyFocusClasses(null)
  })
  if (selectedId.value) {
    cy.getElementById(selectedId.value).select()
    applyFocusClasses(null)
  }
}

async function loadGraph() {
  const version = ++requestVersion
  if (scope.value === 'local' && !props.centerId) {
    graph.value = null
    selectedId.value = null
    cy?.destroy()
    cy = null
    error.value = ''
    isStale.value = false
    return
  }
  loading.value = true
  error.value = ''
  isStale.value = false
  try {
    const result = await requestJson<GraphSnapshot>(withQuery('/api/admin/knowledge/graph', {
      scope: scope.value,
      limit: 500,
      ...(scope.value === 'local' ? { center_id: props.centerId ?? '', depth: depth.value } : {}),
    }))
    if (version !== requestVersion) return
    graph.value = result
    selectedEdgeId.value = null
    if (!visibleNodes.value.some(n => n.id === selectedId.value)) {
      selectedId.value = visibleNodes.value.find(n => n.id === props.centerId)?.id ?? visibleNodes.value[0]?.id ?? null
    }
    await nextTick()
    if (version === requestVersion) draw()
  } catch (cause) {
    if (version !== requestVersion) return
    error.value = formatApiError(cause)
    isStale.value = isApiError(cause) && cause.code === 'index_stale'
    graph.value = null
    cy?.destroy()
    cy = null
  } finally {
    if (version === requestVersion) loading.value = false
  }
}
watch([scope, depth, () => props.centerId, () => props.reloadToken], () => { void loadGraph() })
watch(dark, () => { if (graph.value) draw() })
onMounted(() => { void loadGraph() })
onBeforeUnmount(() => { requestVersion++; cy?.destroy(); cy = null })
</script>

<template>
  <div class="flex min-h-[500px] flex-1 flex-col gap-3">
    <div class="flex flex-wrap items-center gap-2">
      <Button type="button" size="sm" :variant="scope === 'global' ? 'default' : 'outline'"
        @click="scope = 'global'">{{ t('knowledgeV3.graphGlobal') }}</Button>
      <Button type="button" size="sm" :variant="scope === 'local' ? 'default' : 'outline'"
        :disabled="!centerId" @click="scope = 'local'">{{ t('knowledgeV3.graphLocal') }}</Button>
      <label v-if="scope === 'local'" class="flex items-center gap-2 text-xs">
        {{ t('knowledgeV3.graphDepth') }}
        <select v-model.number="depth" class="admin-input w-20"><option :value="1">1</option><option :value="2">2</option></select>
      </label>
      <Button type="button" size="sm" variant="outline" @click="void loadGraph()">{{ t('knowledgeV3.graphRefresh') }}</Button>
      <Button type="button" size="sm" variant="outline" @click="fitGraph">{{ t('knowledgeV3.graphFit') }}</Button>
      <Button type="button" size="sm" variant="outline" @click="relayout">{{ t('knowledgeV3.graphRelayout') }}</Button>
      <span v-if="graph" class="ml-auto text-xs text-muted-foreground">
        {{ visibleNodes.length }} nodes · {{ visibleEdges.length }} links
      </span>
      <span v-if="graph?.truncated" class="text-xs text-amber-600">{{ t('knowledgeV3.graphTruncated') }}</span>
    </div>
    <p v-if="error" role="alert" class="rounded-lg border border-danger/30 p-3 text-sm text-danger">{{ error }}</p>
    <Button v-if="isStale" type="button" size="sm" variant="outline" @click="emit('rebuild')">
      {{ t('knowledgeV3.graphRebuild') }}
    </Button>
    <p v-if="scope === 'local' && !centerId" class="text-sm text-muted-foreground">{{ t('knowledgeV3.graphChooseWiki') }}</p>
    <div class="grid min-h-[410px] flex-1 gap-3 lg:grid-cols-[minmax(0,1fr)_235px]">
      <div
        class="relative flex min-h-[410px] flex-col overflow-hidden rounded-lg border bg-muted/20"
        style="background-image: radial-gradient(color-mix(in srgb, var(--foreground) 10%, transparent) 1px, transparent 1px); background-size: 22px 22px;"
      >
        <div ref="canvas" role="img" :aria-label="t('knowledgeV3.graph')"
          class="min-h-[410px] w-full flex-1"></div>
        <div class="pointer-events-none absolute left-3 top-3 flex flex-wrap items-center gap-2.5 rounded-md border bg-background/85 px-2.5 py-1 text-[11px] text-muted-foreground backdrop-blur-sm">
          <span class="inline-flex items-center gap-1.5"><span class="size-2 rounded-full bg-[#a882ff]"></span>Concept</span>
          <span class="inline-flex items-center gap-1.5"><span class="size-2 rounded-full bg-[#5ccfe6]"></span>Entity</span>
          <span class="inline-flex items-center gap-1.5"><span class="size-2 rounded-full bg-[#ffcc66]"></span>Synthesis</span>
          <span class="inline-flex items-center gap-1.5"><span class="size-2 rounded-full bg-[#ff7a90]"></span>Source</span>
          <span class="inline-flex items-center gap-1.5"><span class="size-2 rounded-full bg-[#3dd68c]"></span>Selected</span>
        </div>
        <p v-if="hoverText" class="pointer-events-none absolute bottom-3 left-3 max-w-[95%] rounded-md border bg-background/95 px-2.5 py-1.5 text-xs shadow">
          {{ hoverText }}
        </p>
        <div v-if="loading" role="status" aria-busy="true" class="card-shimmer pointer-events-none absolute inset-0 bg-muted/15">
          <p class="absolute left-3 top-11 rounded bg-background/90 p-2 text-xs">
            {{ t('knowledgeV3.graphLoading') }}
          </p>
        </div>
        <p v-else-if="graph && !visibleNodes.length" class="pointer-events-none absolute left-3 top-11 text-xs">
          {{ t('knowledgeV3.graphEmpty') }}
        </p>
      </div>
      <aside class="max-h-[540px] space-y-3 overflow-auto rounded-lg border p-3 text-xs">
        <p class="font-semibold leading-snug">{{ current?.title ?? t('knowledgeV3.graphChooseWiki') }}</p>
        <p v-if="current" class="break-all text-muted-foreground">{{ current.path }}</p>
        <Button v-if="current" type="button" variant="outline" size="sm" @click="emit('open', current.id)">
          {{ t('knowledgeV3.graphOpen') }}
        </Button>
        <div v-if="selectedEdge" class="space-y-2 border-b pb-3">
          <p class="font-semibold">{{ t('knowledgeV3.graphLinkDetails') }}</p>
          <p>{{ name(selectedEdge.source_id) }} → {{ name(selectedEdge.target_id) }}</p>
          <p class="text-muted-foreground">{{ selectedEdge.kind }}</p>
          <div v-for="(reference, index) in selectedEdge.references" :key="index" class="rounded bg-muted/40 p-2">
            <p>{{ t('knowledgeV3.graphLine') }} {{ reference.line }}: {{ reference.raw_target }}</p>
            <p v-if="reference.alias">{{ t('knowledgeV3.graphAlias') }}: {{ reference.alias }}</p>
            <p v-if="reference.anchor">{{ t('knowledgeV3.graphAnchor') }}: {{ reference.anchor }}</p>
          </div>
          <Button type="button" variant="outline" size="sm" @click="emit('open', selectedEdge.source_id)">
            {{ t('knowledgeV3.graphOpenSource') }}
          </Button>
        </div>
        <div v-if="current">
          <p class="font-semibold">{{ t('knowledgeV3.graphOutgoing') }} ({{ outgoing.length }})</p>
          <button v-for="e in outgoing" :key="e.target_id + e.kind" type="button"
            class="block w-full rounded px-2 py-1 text-left hover:bg-accent"
            @click="focusEdge(e)">
            → {{ name(e.target_id) }} · {{ e.lines.join(', ') }}
          </button>
          <p class="mt-3 font-semibold">{{ t('knowledgeV3.graphBacklinks') }} ({{ incoming.length }})</p>
          <button v-for="e in incoming" :key="e.source_id + e.kind" type="button"
            class="block w-full rounded px-2 py-1 text-left hover:bg-accent"
            @click="focusEdge(e)">
            ← {{ name(e.source_id) }} · {{ e.lines.join(', ') }}
          </button>
          <p class="mt-3 font-semibold">{{ t('knowledgeV3.graphUnresolved') }} ({{ unresolved.length }})</p>
          <p v-for="e in unresolved" :key="e.source_line + e.raw_target" class="break-all text-amber-600">
            {{ e.source_line }}: {{ e.raw_target }} ({{ e.reason }})
          </p>
        </div>
        <div class="border-t pt-2">
          <p class="mb-2 font-medium">{{ t('knowledgeV3.graphNodes') }}</p>
          <button v-for="node in visibleNodes" :key="node.id" type="button"
            class="block w-full truncate rounded px-2 py-1 text-left hover:bg-accent"
            :class="selectedId === node.id ? 'bg-accent font-semibold' : ''"
            @click="focusNode(node.id)">
            {{ node.title }} <span v-if="node.status === 'draft'" class="text-amber-600">●</span>
          </button>
        </div>
      </aside>
    </div>
  </div>
</template>

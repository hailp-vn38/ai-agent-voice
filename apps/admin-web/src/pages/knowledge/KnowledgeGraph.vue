<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import cytoscape, { type Core } from 'cytoscape'
import {
  Globe,
  Crosshair,
  Maximize2,
  Minimize2,
  RefreshCw,
  RotateCw,
  Search,
  SlidersHorizontal,
  Scan,
} from '@lucide/vue'
import { requestJson, withQuery } from '@/api/client'
import { formatApiError, isApiError } from '@/api/errors'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useTheme } from '@/composables/useTheme'
import './knowledge-reader.css'

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

const MIN_ZOOM = 0.28
const MAX_ZOOM = 2.4
const DEFAULT_MAX_FIT_ZOOM = 1.12

const props = defineProps<{
  centerId?: string | null
  reloadToken: number
  focusMode?: boolean
}>()
const emit = defineEmits<{
  open: [id: string]
  rebuild: []
  'update:focusMode': [value: boolean]
}>()

const { t } = useI18n()
const { dark } = useTheme()

const localFocusMode = ref(Boolean(props.focusMode))
watch(() => props.focusMode, value => {
  if (typeof value === 'boolean') localFocusMode.value = value
})
const isFocusMode = computed({
  get: () => localFocusMode.value,
  set: (value: boolean) => {
    localFocusMode.value = value
    emit('update:focusMode', value)
  },
})

const scope = ref<'global' | 'local'>('global')
const depth = ref(1)
const graph = ref<GraphSnapshot | null>(null)
const selectedId = ref<string | null>(null)
const activeId = ref<string | null>(null)
const selectedEdgeId = ref<string | null>(null)
const loading = ref(false)
const error = ref('')
const isStale = ref(false)
const hoverText = ref('')
const isDraggingCanvas = ref(false)

// Prototype display & search controls
const showAllLabels = ref(false)
const focusNeighbors = ref(true)
const showLegend = ref(true)
const nodeSizePercent = ref(100)
const viewMenuOpen = ref(false)
const nodeSearchOpen = ref(false)
const nodeSearchQuery = ref('')
const nodeSearchInput = ref<HTMLInputElement | null>(null)

const canvasWrap = ref<HTMLElement | null>(null)
const canvas = ref<HTMLElement | null>(null)
const inspectPanel = ref<HTMLElement | null>(null)
const inspectCoords = ref<{ left: number; top: number } | null>(null)
const panelFlipped = ref(false)

let cy: Core | null = null
let requestVersion = 0
let layoutRunCount = 0
let zoomRafId: number | null = null
let targetZoom = 1
let zoomAnchor = { x: 0, y: 0 }
let resizeObserver: ResizeObserver | null = null

const nodeSizeScale = computed(() => nodeSizePercent.value / 100)

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

function getPalette() {
  return dark.value
    ? {
        canvasBg: '#101420',
        text: '#eaeef9',
        edge: '#687394',
        edgeActive: '#b3a2ff',
        selectedRing: '#ffffff',
        concept: '#b38aff',
        entity: '#54c7dc',
        synthesis: '#ffc765',
        source: '#fa7294',
        system: '#a3b1c7',
        draft: '#f59e0b',
      }
    : {
        canvasBg: '#f6f7fb',
        text: '#181c2e',
        edge: '#8e98b8',
        edgeActive: '#5c3cf6',
        selectedRing: '#4f35d2',
        concept: '#6941f5',
        entity: '#0891b2',
        synthesis: '#d97706',
        source: '#e11d48',
        system: '#64748b',
        draft: '#d97706',
      }
}

function groupColor(group: NodeGroup): string {
  const p = getPalette()
  return p[group] ?? p.concept
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

const effectiveSelectedId = computed(() => activeId.value ?? selectedId.value)
const current = computed(() => visibleNodes.value.find(node => node.id === effectiveSelectedId.value) ?? null)
const inspectorOpen = computed(() => Boolean(activeId.value || selectedEdgeId.value))

const selectedEdge = computed(() => {
  if (!selectedEdgeId.value) return null
  const index = Number(selectedEdgeId.value.replace('link_', ''))
  return visibleEdges.value[index] ?? null
})
const outgoing = computed(() => visibleEdges.value.filter(e => e.source_id === effectiveSelectedId.value))
const incoming = computed(() => visibleEdges.value.filter(e => e.target_id === effectiveSelectedId.value))
const unresolved = computed(() => (graph.value?.unresolved ?? []).filter(e => e.source_id === effectiveSelectedId.value))
const connectedNodeIds = computed(() => {
  const id = effectiveSelectedId.value
  if (!id) return []
  const set = new Set<string>()
  for (const e of visibleEdges.value) {
    if (e.source_id === id && e.target_id !== id) set.add(e.target_id)
    if (e.target_id === id && e.source_id !== id) set.add(e.source_id)
  }
  return [...set]
})

const names = computed(() => new Map((graph.value?.nodes ?? []).map(n => [n.id, n.title])))
function name(id: string): string { return names.value.get(id) ?? id }

const graphHeadingText = computed(() => {
  if (activeId.value || selectedEdgeId.value) return 'Node connections'
  if (scope.value === 'local') return 'Current note'
  return 'All connections'
})

const filteredSearchNodes = computed(() => {
  const q = nodeSearchQuery.value.trim().toLowerCase()
  const list = visibleNodes.value
  if (!q) return list.slice(0, 9)
  return list
    .filter(n => n.title.toLowerCase().includes(q) || n.path.toLowerCase().includes(q))
    .slice(0, 9)
})

const inspectStyle = computed<Record<string, string>>(() => {
  if (isFocusMode.value || !inspectCoords.value) return {} as Record<string, string>
  return {
    left: `${inspectCoords.value.left}px`,
    top: `${inspectCoords.value.top}px`,
  }
})

function placePanel() {
  if (isFocusMode.value || !inspectorOpen.value) {
    inspectCoords.value = null
    panelFlipped.value = false
    return
  }
  const targetId = activeId.value ?? selectedEdge.value?.source_id ?? selectedId.value
  if (!targetId || !cy || typeof cy.getElementById !== 'function' || !canvasWrap.value) return
  const target = cy.getElementById(targetId)
  if (!target || typeof target.renderedPosition !== 'function') return
  const pos = target.renderedPosition()
  if (!pos || Number.isNaN(pos.x) || Number.isNaN(pos.y)) return
  const radius = typeof target.renderedOuterWidth === 'function' ? target.renderedOuterWidth() / 2 : 14
  const cw = canvasWrap.value.clientWidth || 800
  const ch = canvasWrap.value.clientHeight || 600
  const pw = inspectPanel.value?.offsetWidth || 292
  const ph = inspectPanel.value?.offsetHeight || 290

  let left = pos.x + radius + 18
  let flipped = false
  if (left + pw > cw - 16) {
    left = pos.x - radius - pw - 18
    flipped = true
  }
  left = Math.max(12, Math.min(left, cw - pw - 12))
  const top = Math.max(76, Math.min(pos.y - 26, ch - ph - 74))
  panelFlipped.value = flipped
  inspectCoords.value = { left: Math.round(left), top: Math.round(top) }
}

function stopSmoothZoom() {
  if (zoomRafId !== null) {
    cancelAnimationFrame(zoomRafId)
    zoomRafId = null
  }
}

function getViewportCenter() {
  const fullW = canvas.value?.clientWidth || canvasWrap.value?.clientWidth || 800
  const fullH = canvas.value?.clientHeight || canvasWrap.value?.clientHeight || 560
  const drawerOffset = isFocusMode.value && inspectorOpen.value ? 176 : 0
  return {
    x: Math.max(160, (fullW - drawerOffset) / 2),
    y: fullH / 2,
  }
}

function clampViewportZoom() {
  if (!cy || typeof cy.zoom !== 'function' || typeof cy.center !== 'function') return
  const currentZoom = cy.zoom()
  if (currentZoom > DEFAULT_MAX_FIT_ZOOM) {
    const center = getViewportCenter()
    cy.zoom({
      level: DEFAULT_MAX_FIT_ZOOM,
      renderedPosition: center,
    })
  }
  cy.center()
  targetZoom = cy.zoom()
}

function stepSmoothZoom() {
  if (!cy || typeof cy.zoom !== 'function') {
    zoomRafId = null
    return
  }
  const currentZoom = cy.zoom()
  const diff = targetZoom - currentZoom
  if (Math.abs(diff) < 0.0008) {
    cy.zoom({ level: targetZoom, renderedPosition: zoomAnchor })
    zoomRafId = null
    applyFocusClasses( null )
    placePanel()
    return
  }
  const nextZoom = currentZoom + diff * 0.28
  cy.zoom({ level: nextZoom, renderedPosition: zoomAnchor })
  placePanel()
  zoomRafId = requestAnimationFrame(stepSmoothZoom)
}

function scheduleSmoothZoom() {
  if (!cy || typeof cy.zoom !== 'function') return
  if (zoomRafId === null) {
    zoomRafId = requestAnimationFrame(stepSmoothZoom)
  }
}

function onCanvasWheel(event: WheelEvent) {
  if (!cy || typeof cy.zoom !== 'function' || !canvas.value) return
  const rawDelta =
    event.deltaMode === 1
      ? event.deltaY * 18
      : event.deltaMode === 2
        ? event.deltaY * 120
        : event.deltaY
  const clampedDelta = Math.max(-85, Math.min(85, rawDelta))
  const sensitivity = event.ctrlKey || event.metaKey ? 0.0042 : 0.0022
  const factor = Math.exp(-clampedDelta * sensitivity)
  if (zoomRafId === null) {
    targetZoom = cy.zoom()
  }
  targetZoom = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, targetZoom * factor))
  const rect = canvas.value.getBoundingClientRect()
  zoomAnchor = {
    x: event.clientX - rect.left,
    y: event.clientY - rect.top,
  }
  scheduleSmoothZoom()
}

function zoomByFactor(factor: number) {
  if (!cy || typeof cy.zoom !== 'function') return
  if (zoomRafId === null) {
    targetZoom = cy.zoom()
  }
  targetZoom = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, targetZoom * factor))
  zoomAnchor = getViewportCenter()
  scheduleSmoothZoom()
}

function computeObsidianPositions(
  nodes: GraphNode[],
  edges: GraphEdge[],
  seedOffset = 0,
): Record<string, { x: number; y: number }> {
  const n = nodes.length
  if (n === 0) return {}
  if (n === 1) return { [nodes[0]!.id]: { x: 0, y: 0 } }

  const containerW = Math.max(480, canvas.value?.clientWidth || canvasWrap.value?.clientWidth || 860)
  const containerH = Math.max(400, canvas.value?.clientHeight || canvasWrap.value?.clientHeight || 580)
  const aspectX = Math.max(1.0, Math.min(1.38, containerW / containerH))

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
  const focalId = (scope.value === 'local' ? props.centerId : null) ?? effectiveSelectedId.value
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
    const r1 = Math.max(135, ring1.length * 26)
    ring1.forEach((idx, k) => {
      const angle = baseAngle + (2 * Math.PI * k) / Math.max(1, ring1.length)
      x[idx] = Math.cos(angle) * r1 * aspectX
      y[idx] = Math.sin(angle) * r1
    })
    const r2 = r1 + Math.max(125, ring2.length * 22)
    ring2.forEach((idx, k) => {
      const parents = adj[idx]!.filter(nb => hop[nb] === 1)
      if (parents.length > 0) {
        const px = parents.reduce((acc, p) => acc + x[p]!, 0) / parents.length
        const py = parents.reduce((acc, p) => acc + y[p]!, 0) / parents.length
        const pAngle = Math.atan2(py, px / aspectX) + ((k % 3) - 1) * 0.22
        x[idx] = Math.cos(pAngle) * r2 * aspectX
        y[idx] = Math.sin(pAngle) * r2
      } else {
        const angle = baseAngle + (2 * Math.PI * (k + 0.5)) / Math.max(1, ring2.length)
        x[idx] = Math.cos(angle) * r2 * aspectX
        y[idx] = Math.sin(angle) * r2
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
      .sort((a, b) => b.deg - a.deg || a.g - b.g)

    ordered.forEach((item, rank) => {
      if (item.deg === maxDeg && rank === 0) {
        x[item.i] = 0
        y[item.i] = 0
        return
      }
      const sectorAngle =
        seedOffset * 0.5 + (2 * Math.PI * item.g) / Math.max(1, groups.length)
      const jitter = ((rank % 5) - 2) * 0.22
      const angle = sectorAngle + jitter
      const radius = 92 + (rank % 4) * 56
      x[item.i] = Math.cos(angle) * radius * aspectX
      y[item.i] = Math.sin(angle) * radius
    })

    const ticks = 150
    for (let step = 0; step < ticks; step++) {
      const alpha = 1 - step / ticks
      const fx = new Float64Array(n)
      const fy = new Float64Array(n)

      for (let i = 0; i < n; i++) {
        for (let j = i + 1; j < n; j++) {
          let dx = (x[j]! - x[i]!) / aspectX
          let dy = y[j]! - y[i]!
          let d2 = dx * dx + dy * dy
          if (d2 < 1) {
            dx = (i - j) * 0.5
            dy = 0.5
            d2 = dx * dx + dy * dy
          }
          const d = Math.sqrt(d2)
          const repulse = (9200 * alpha) / d2
          const ux = (dx / d) * repulse * aspectX
          const uy = (dy / d) * repulse
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
        const spring = (d - 115) * 0.19 * weight * alpha
        const ux = (dx / d) * spring
        const uy = (dy / d) * spring
        fx[u]! += ux
        fy[u]! += uy
        fx[v]! -= ux
        fy[v]! -= uy
      }

      for (let i = 0; i < n; i++) {
        fx[i]! -= (x[i]! / aspectX) * 0.028 * alpha
        fy[i]! -= y[i]! * 0.028 * alpha
        x[i]! += Math.max(-28, Math.min(28, fx[i]!))
        y[i]! += Math.max(-28, Math.min(28, fy[i]!))
      }
    }
  }

  // Anisotropic label-box collision resolution tailored to viewport aspect ratio.
  const minX = scope.value === 'local' ? 116 : 98
  const minY = scope.value === 'local' ? 64 : 60
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

  // Recenter bounding box around (0, 0) so the graph is always dead-center in the canvas.
  if (scope.value !== 'local' || centerIdx === undefined) {
    let minBx = Infinity
    let maxBx = -Infinity
    let minBy = Infinity
    let maxBy = -Infinity
    for (let i = 0; i < n; i++) {
      if (x[i]! < minBx) minBx = x[i]!
      if (x[i]! > maxBx) maxBx = x[i]!
      if (y[i]! < minBy) minBy = y[i]!
      if (y[i]! > maxBy) maxBy = y[i]!
    }
    const cx = (minBx + maxBx) / 2
    const cyOffset = (minBy + maxBy) / 2
    for (let i = 0; i < n; i++) {
      x[i] = x[i]! - cx
      y[i] = y[i]! - cyOffset
    }
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
    padding: 54,
    positions: (node: { id: () => string }) => positions[node.id()] ?? { x: 0, y: 0 },
  }
}

function fitGraph() {
  stopSmoothZoom()
  cy?.fit(undefined, 54)
  clampViewportZoom()
  requestAnimationFrame(placePanel)
}

function relayout() {
  stopSmoothZoom()
  layoutRunCount++
  cy?.layout(buildLayoutOptions(visibleNodes.value, visibleEdges.value, layoutRunCount)).run()
  clampViewportZoom()
  requestAnimationFrame(placePanel)
}

function selectScope(nextScope: 'global' | 'local') {
  if (nextScope === 'local' && !props.centerId) return
  scope.value = nextScope
  if (nextScope === 'local' && props.centerId) {
    selectedId.value = props.centerId
    activeId.value = props.centerId
  }
}

function clearSelection() {
  activeId.value = null
  selectedEdgeId.value = null
  if (cy && typeof cy.elements === 'function') {
    cy.elements().unselect()
  }
  applyFocusClasses(null)
  placePanel()
}

function chooseNode(id: string, keepOpen = false) {
  selectedEdgeId.value = null
  if (!keepOpen && activeId.value === id && !isFocusMode.value) {
    clearSelection()
    return
  }
  selectedId.value = id
  activeId.value = id
  cy?.getElementById(id).select()
  applyFocusClasses(null)
  void nextTick(() => requestAnimationFrame(placePanel))
}

function chooseNodeFromSearch(id: string) {
  nodeSearchOpen.value = false
  chooseNode(id, true)
  if (cy && typeof cy.getElementById === 'function' && typeof cy.center === 'function') {
    const el = cy.getElementById(id)
    if (el) cy.center(el)
    requestAnimationFrame(placePanel)
  }
}

function focusEdge(edge: GraphEdge) {
  const index = visibleEdges.value.indexOf(edge)
  if (index < 0) return
  selectedEdgeId.value = 'link_' + index
  if (!activeId.value) {
    activeId.value = selectedId.value ?? edge.source_id
  }
  cy?.getElementById(selectedEdgeId.value).select()
  applyFocusClasses(null)
  void nextTick(() => requestAnimationFrame(placePanel))
}

function isolateCurrentNode() {
  focusNeighbors.value = true
  if (current.value && cy && typeof cy.getElementById === 'function' && typeof cy.center === 'function') {
    const el = cy.getElementById(current.value.id)
    if (el) cy.center(el)
  }
  applyFocusClasses(null)
  requestAnimationFrame(placePanel)
}

function toggleFocusMode() {
  isFocusMode.value = !isFocusMode.value
}

function toggleViewMenu() {
  viewMenuOpen.value = !viewMenuOpen.value
  if (viewMenuOpen.value) nodeSearchOpen.value = false
}

function toggleNodeSearch() {
  nodeSearchOpen.value = !nodeSearchOpen.value
  if (nodeSearchOpen.value) {
    viewMenuOpen.value = false
    nodeSearchQuery.value = ''
    void nextTick(() => nodeSearchInput.value?.focus())
  }
}

function applyFocusClasses(hoveredId: string | null) {
  if (!cy || typeof cy.elements !== 'function') return
  const all = cy.elements()
  all.removeClass('is-dimmed is-highlighted is-active-edge is-show-label')

  const zoomedIn = typeof cy.zoom === 'function' && cy.zoom() >= 1.22
  if (showAllLabels.value || zoomedIn || scope.value === 'local') {
    cy.nodes().addClass('is-show-label')
  }

  const focalNodeId = hoveredId ?? activeId.value
  if (focalNodeId) {
    const target = cy.getElementById(focalNodeId)
    if (target && typeof target.closedNeighborhood === 'function') {
      const neighborhood = target.closedNeighborhood()
      if (focusNeighbors.value) {
        all.not(neighborhood).addClass('is-dimmed')
      }
      neighborhood.addClass('is-highlighted is-show-label')
      if (typeof target.connectedEdges === 'function') {
        target.connectedEdges().addClass('is-active-edge')
      }
    }
  }
}

function draw() {
  stopSmoothZoom()
  cy?.destroy()
  cy = null
  if (!canvas.value || !graph.value) return
  const nodes = visibleNodes.value
  const edges = visibleEdges.value

  const degreeById = new Map<string, number>()
  for (const n of nodes) degreeById.set(n.id, 0)
  for (const e of edges) {
    degreeById.set(e.source_id, (degreeById.get(e.source_id) ?? 0) + 1)
    degreeById.set(e.target_id, (degreeById.get(e.target_id) ?? 0) + 1)
  }

  const palette = getPalette()
  const isLocal = scope.value === 'local'
  const baseFontSize = isLocal ? 11.5 : 12.5
  const scale = nodeSizeScale.value

  const elements = [
    ...nodes.map(n => {
      const deg = degreeById.get(n.id) ?? 0
      const baseSize = Math.min(26, (isLocal ? 13 : 11) + Math.round(Math.sqrt(deg) * 3.2))
      const size = Math.max(8, Math.round(baseSize * scale))
      const group = classifyNodeGroup(n)
      const isMinor = !isLocal && nodes.length > 14 && deg <= 1 ? 'yes' : 'no'
      return {
        data: {
          id: n.id,
          title: n.title,
          shortLabel: compactNodeLabel(n.title, isLocal ? 26 : 22),
          selectedLabel: compactNodeLabel(n.title, 38),
          status: n.status,
          group,
          size,
          minor: isMinor,
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
    minZoom: MIN_ZOOM,
    maxZoom: MAX_ZOOM,
    userZoomingEnabled: false,
    userPanningEnabled: true,
    boxSelectionEnabled: false,
    style: [
      {
        selector: 'node',
        style: {
          'label': 'data(shortLabel)',
          'font-size': baseFontSize,
          'font-weight': 560,
          'text-wrap': 'wrap',
          'text-max-width': isLocal ? '136px' : '128px',
          'text-valign': 'bottom',
          'text-margin-y': 6,
          'color': palette.text,
          'text-outline-width': 3.2,
          'text-outline-color': palette.canvasBg,
          'text-outline-opacity': 0.94,
          'background-color': palette.concept,
          'width': 'data(size)',
          'height': 'data(size)',
          'border-width': 2.2,
          'border-color': palette.canvasBg,
          'border-opacity': 0.95,
          'transition-property': 'opacity, width, height, border-width, border-color',
          'transition-duration': 140,
        },
      },
      {
        selector: 'node[minor = "yes"]',
        style: {
          'text-opacity': 0,
        },
      },
      {
        selector: 'node.is-show-label',
        style: {
          'text-opacity': 1,
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
          'text-opacity': 1,
          'text-max-width': '172px',
          'border-width': 3.5,
          'border-color': palette.selectedRing,
          'border-opacity': 1,
          'font-weight': 700,
          'z-index': 20,
        },
      },
      {
        selector: 'edge',
        style: {
          'width': isLocal ? 1.25 : 1.05,
          'line-color': palette.edge,
          'target-arrow-color': palette.edge,
          'target-arrow-shape': isLocal ? 'vee' : 'none',
          'arrow-scale': 0.65,
          'curve-style': 'straight',
          'opacity': isLocal ? 0.38 : 0.22,
          'transition-property': 'opacity, width, line-color, target-arrow-color',
          'transition-duration': 140,
        },
      },
      {
        selector: 'edge.is-active-edge',
        style: {
          'width': 1.85,
          'line-color': palette.edgeActive,
          'target-arrow-color': palette.edgeActive,
          'target-arrow-shape': 'vee',
          'arrow-scale': 0.72,
          'opacity': 0.86,
          'z-index': 12,
        },
      },
      {
        selector: 'edge:selected',
        style: {
          'width': 2.2,
          'line-color': palette.edgeActive,
          'target-arrow-color': palette.edgeActive,
          'target-arrow-shape': 'vee',
          'opacity': 0.96,
          'z-index': 15,
        },
      },
      {
        selector: 'node.is-dimmed',
        style: {
          'opacity': 0.12,
        },
      },
      {
        selector: 'edge.is-dimmed',
        style: {
          'opacity': 0.035,
        },
      },
      {
        selector: 'node.is-highlighted',
        style: {
          'opacity': 1,
          'text-opacity': 1,
          'z-index': 18,
        },
      },
      {
        selector: 'edge.is-highlighted',
        style: {
          'opacity': 0.88,
          'width': 1.85,
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
    chooseNode(event.target.id())
  })
  cy.on('tap', 'edge', event => {
    selectedEdgeId.value = event.target.id()
    if (!activeId.value) {
      activeId.value = selectedId.value
    }
    applyFocusClasses(null)
    void nextTick(() => requestAnimationFrame(placePanel))
  })
  cy.on('tap', '', event => {
    if (event.target === cy) {
      viewMenuOpen.value = false
      nodeSearchOpen.value = false
      clearSelection()
    }
  })
  cy.on('pan zoom', '', () => {
    if (inspectorOpen.value && !isFocusMode.value) {
      requestAnimationFrame(placePanel)
    }
  })
  cy.on('mousedown', '', () => { isDraggingCanvas.value = true })
  cy.on('mouseup', '', () => { isDraggingCanvas.value = false })
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

  if (activeId.value) {
    cy.getElementById(activeId.value).select()
  }
  applyFocusClasses(null)
  requestAnimationFrame(placePanel)
}

async function loadGraph() {
  const version = ++requestVersion
  if (scope.value === 'local' && !props.centerId) {
    graph.value = null
    selectedId.value = null
    activeId.value = null
    selectedEdgeId.value = null
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
    if (activeId.value && !visibleNodes.value.some(n => n.id === activeId.value)) {
      activeId.value = scope.value === 'local' ? selectedId.value : null
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

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    viewMenuOpen.value = false
    nodeSearchOpen.value = false
    if (inspectorOpen.value) clearSelection()
    return
  }
  const tag = (document.activeElement as HTMLElement | null)?.tagName
  if (tag && /INPUT|TEXTAREA|SELECT/.test(tag)) return
  if (event.key === '/') {
    event.preventDefault()
    toggleNodeSearch()
  }
}

watch([scope, depth, () => props.centerId, () => props.reloadToken], () => { void loadGraph() })
watch(dark, () => { if (graph.value) draw() })
watch([showAllLabels, focusNeighbors], () => { applyFocusClasses(null) })
watch(nodeSizePercent, () => { if (graph.value) draw() })
watch(isFocusMode, () => {
  void nextTick(() => {
    if (cy && typeof cy.resize === 'function') cy.resize()
    placePanel()
  })
})

onMounted(() => {
  void loadGraph()
  window.addEventListener('keydown', onKeydown)
  if (typeof ResizeObserver !== 'undefined' && canvasWrap.value) {
    resizeObserver = new ResizeObserver(() => {
      if (cy && typeof cy.resize === 'function') {
        cy.resize()
      }
      placePanel()
    })
    resizeObserver.observe(canvasWrap.value)
  }
})

onBeforeUnmount(() => {
  requestVersion++
  stopSmoothZoom()
  window.removeEventListener('keydown', onKeydown)
  resizeObserver?.disconnect()
  resizeObserver = null
  cy?.destroy()
  cy = null
})
</script>

<template>
  <div class="kg-root" :class="{ 'is-focus-mode': isFocusMode }">
    <div v-if="error || isStale || (scope === 'local' && !centerId)" class="kg-alert-strip">
      <p v-if="error" role="alert" class="kg-alert-error">{{ error }}</p>
      <Button v-if="isStale" type="button" size="sm" variant="outline" @click="emit('rebuild')">
        {{ t('knowledgeV3.graphRebuild') }}
      </Button>
      <p v-if="scope === 'local' && !centerId" class="text-xs text-muted-foreground">
        {{ t('knowledgeV3.graphChooseWiki') }}
      </p>
    </div>

    <div ref="canvasWrap" class="kg-canvas-wrap" :class="{ 'is-dragging': isDraggingCanvas }">
      <!-- Cytoscape Surface (keeps flex-1 & min-h-[410px] without absolute) -->
      <div
        ref="canvas"
        role="img"
        :aria-label="t('knowledgeV3.graph')"
        class="kg-canvas min-h-[410px] w-full flex-1"
        @wheel.prevent="onCanvasWheel"
      ></div>

      <!-- Top Floating Toolbar (Prototype .toptools) -->
      <div class="kg-toptools">
        <div class="kg-heading">
          <span class="kg-overline">KNOWLEDGE / GRAPH</span>
          <strong class="kg-heading-title">{{ graphHeadingText }}</strong>
        </div>

        <div class="kg-tools">
          <button
            type="button"
            class="kg-tool-btn"
            :class="{ on: scope === 'global' }"
            @click="selectScope('global')"
          >
            <Globe :size="13" />
            <span>{{ t('knowledgeV3.graphGlobal') }}</span>
          </button>
          <button
            type="button"
            class="kg-tool-btn"
            :class="{ on: scope === 'local' }"
            :disabled="!centerId"
            @click="selectScope('local')"
          >
            <Crosshair :size="13" />
            <span>{{ t('knowledgeV3.graphLocal') }}</span>
          </button>
          <label v-if="scope === 'local'" class="kg-depth-select">
            <span>{{ t('knowledgeV3.graphDepth') }}</span>
            <select v-model.number="depth">
              <option :value="1">1</option>
              <option :value="2">2</option>
            </select>
          </label>
          <span class="kg-sep"></span>
          <button type="button" class="kg-tool-btn kg-word" @click="fitGraph">
            <Scan :size="13" />
            <span>{{ t('knowledgeV3.graphFit') }}</span>
          </button>
          <button type="button" class="kg-tool-btn kg-word" @click="relayout">
            <RefreshCw :size="13" />
            <span>{{ t('knowledgeV3.graphRelayout') }}</span>
          </button>
          <button
            type="button"
            class="kg-tool-btn"
            :class="{ on: nodeSearchOpen }"
            title="Search nodes"
            aria-label="Search nodes"
            @click="toggleNodeSearch"
          >
            <Search :size="14" />
          </button>
          <button
            type="button"
            class="kg-tool-btn"
            :class="{ on: viewMenuOpen }"
            title="Display settings"
            aria-label="Display settings"
            @click="toggleViewMenu"
          >
            <SlidersHorizontal :size="13" />
            <span class="kg-word">View</span>
          </button>
        </div>

        <div class="kg-push"></div>

        <span v-if="graph" class="kg-meta">
          {{ visibleNodes.length }} nodes · {{ visibleEdges.length }} links
        </span>
        <span v-if="graph?.truncated" class="kg-truncated">{{ t('knowledgeV3.graphTruncated') }}</span>

        <button
          type="button"
          class="kg-iconbtn"
          :title="t('knowledgeV3.graphRefresh')"
          :aria-label="t('knowledgeV3.graphRefresh')"
          @click="void loadGraph()"
        >
          <RotateCw :size="15" />
        </button>
        <button
          type="button"
          class="kg-iconbtn"
          :class="{ on: isFocusMode }"
          title="Focus mode"
          aria-label="Focus mode"
          :aria-pressed="isFocusMode"
          @click="toggleFocusMode"
        >
          <Minimize2 v-if="isFocusMode" :size="15" />
          <Maximize2 v-else :size="15" />
        </button>
      </div>

      <!-- Display Settings Menu (Prototype #menu) -->
      <div class="kg-menu" :class="{ open: viewMenuOpen }" role="dialog" aria-label="Graph display">
        <b>Graph display</b>
        <label>
          <span>Show all labels</span>
          <input v-model="showAllLabels" type="checkbox" />
        </label>
        <label>
          <span>Focus neighbors</span>
          <input v-model="focusNeighbors" type="checkbox" />
        </label>
        <label>
          <span>Show legend</span>
          <input v-model="showLegend" type="checkbox" />
        </label>
        <label>
          <span>Node size</span>
          <input v-model.number="nodeSizePercent" type="range" min="70" max="140" />
        </label>
      </div>

      <!-- Floating Node Search Palette (Prototype #search) -->
      <div class="kg-search" :class="{ open: nodeSearchOpen }" role="search" aria-label="Search graph nodes">
        <input
          ref="nodeSearchInput"
          v-model="nodeSearchQuery"
          type="search"
          placeholder="Search a node..."
          aria-label="Search a node"
          autocomplete="off"
          @keydown.esc.stop.prevent="nodeSearchOpen = false"
        />
        <div class="kg-search-results">
          <button
            v-for="node in filteredSearchNodes"
            :key="node.id"
            type="button"
            class="kg-search-result"
            @click="chooseNodeFromSearch(node.id)"
          >
            <span class="kg-dot" :style="{ background: groupColor(classifyNodeGroup(node)) }"></span>
            <span class="truncate font-medium">{{ node.title }}</span>
            <span class="kg-search-path">{{ node.path }}</span>
          </button>
          <p v-if="!filteredSearchNodes.length" class="kg-search-empty">
            {{ t('knowledgeV3.noSearchResults') }}
          </p>
        </div>
      </div>

      <!-- Inspector: Option A Floating Context Card (default) + Option B Right Drawer (Focus Mode) -->
      <aside
        ref="inspectPanel"
        class="kg-inspect"
        :class="{
          'is-hidden': !inspectorOpen,
          'is-drawer': isFocusMode,
          'is-flipped': panelFlipped,
        }"
        :style="inspectStyle"
      >
        <div class="kg-inspect-intro">
          <div v-if="current" class="kg-inspect-type">
            <span class="kg-dot" :style="{ background: groupColor(classifyNodeGroup(current)) }"></span>
            <span>{{ classifyNodeGroup(current) }}</span>
          </div>
          <button
            type="button"
            class="kg-inspect-close"
            title="Close"
            aria-label="Close inspector"
            @click="clearSelection"
          >
            &times;
          </button>
          <div class="kg-inspect-name">{{ current?.title ?? t('knowledgeV3.graphChooseWiki') }}</div>
          <div v-if="current" class="kg-inspect-path">{{ current.path }}</div>
        </div>

        <div v-if="current" class="kg-inspect-actions">
          <button type="button" class="kg-btn kg-btn-primary" @click="emit('open', current.id)">
            <span>{{ t('knowledgeV3.graphOpen') }}</span>
            <span aria-hidden="true">&#8599;</span>
          </button>
          <button type="button" class="kg-btn" @click="isolateCurrentNode">
            Focus node
          </button>
        </div>

        <div v-if="selectedEdge" class="kg-edge-details">
          <div class="kg-sect">{{ t('knowledgeV3.graphLinkDetails') }}</div>
          <p class="kg-edge-pair">{{ name(selectedEdge.source_id) }} → {{ name(selectedEdge.target_id) }}</p>
          <p class="kg-edge-kind">{{ selectedEdge.kind }}</p>
          <div
            v-for="(reference, index) in selectedEdge.references"
            :key="index"
            class="kg-edge-ref"
          >
            <p>{{ t('knowledgeV3.graphLine') }} {{ reference.line }}: {{ reference.raw_target }}</p>
            <p v-if="reference.alias">{{ t('knowledgeV3.graphAlias') }}: {{ reference.alias }}</p>
            <p v-if="reference.anchor">{{ t('knowledgeV3.graphAnchor') }}: {{ reference.anchor }}</p>
          </div>
          <button type="button" class="kg-btn kg-edge-open" @click="emit('open', selectedEdge.source_id)">
            {{ t('knowledgeV3.graphOpenSource') }}
          </button>
        </div>

        <div v-if="current" class="kg-inspect-mid">
          <div v-if="connectedNodeIds.length" class="kg-sect">
            Connections ({{ connectedNodeIds.length }})
          </div>
          <div v-if="connectedNodeIds.length" class="kg-links">
            <button
              v-for="id in connectedNodeIds.slice(0, 5)"
              :key="id"
              type="button"
              class="kg-link"
              @click="chooseNode(id, true)"
            >
              &#8599; {{ name(id) }}
            </button>
          </div>

          <div class="kg-sect">{{ t('knowledgeV3.graphOutgoing') }} ({{ outgoing.length }})</div>
          <div class="kg-links">
            <button
              v-for="e in outgoing"
              :key="e.target_id + e.kind"
              type="button"
              class="kg-link"
              @click="focusEdge(e)"
            >
              → {{ name(e.target_id) }} · {{ e.lines.join(', ') }}
            </button>
          </div>

          <div class="kg-sect">{{ t('knowledgeV3.graphBacklinks') }} ({{ incoming.length }})</div>
          <div class="kg-links">
            <button
              v-for="e in incoming"
              :key="e.source_id + e.kind"
              type="button"
              class="kg-link"
              @click="focusEdge(e)"
            >
              ← {{ name(e.source_id) }} · {{ e.lines.join(', ') }}
            </button>
          </div>

          <template v-if="unresolved.length">
            <div class="kg-sect">{{ t('knowledgeV3.graphUnresolved') }} ({{ unresolved.length }})</div>
            <p
              v-for="e in unresolved"
              :key="e.source_line + e.raw_target"
              class="break-all text-[11px] text-amber-500"
            >
              {{ e.source_line }}: {{ e.raw_target }} ({{ e.reason }})
            </p>
          </template>
        </div>
      </aside>

      <!-- Bottom-Left Category Legend (Prototype #legend) -->
      <div v-show="showLegend" class="kg-legend">
        <span><i :style="{ background: groupColor('concept') }"></i>Concept</span>
        <span><i :style="{ background: groupColor('entity') }"></i>Entity</span>
        <span><i :style="{ background: groupColor('synthesis') }"></i>Synthesis</span>
        <span><i :style="{ background: groupColor('source') }"></i>Source</span>
      </div>

      <!-- Bottom-Right Smooth Zoom Controls (Prototype .zoom) -->
      <div class="kg-zoom">
        <button type="button" title="Zoom out" aria-label="Zoom out" @click="zoomByFactor(1 / 1.22)">
          &#8722;
        </button>
        <button type="button" title="Zoom in" aria-label="Zoom in" @click="zoomByFactor(1.22)">
          +
        </button>
        <button
          type="button"
          :title="t('knowledgeV3.graphFit')"
          :aria-label="t('knowledgeV3.graphFit')"
          @click="fitGraph"
        >
          &#9635;
        </button>
      </div>

      <!-- Bottom-Center Help / Hover Pill (Prototype #help) -->
      <div v-if="hoverText || !inspectorOpen" class="kg-help">
        {{ hoverText || 'Click any node to inspect · Drag to pan · Scroll to zoom' }}
      </div>

      <!-- Loading & Empty states -->
      <div
        v-if="loading"
        role="status"
        aria-busy="true"
        class="card-shimmer pointer-events-none absolute inset-0 bg-muted/10"
      >
        <p class="kg-status-pill">
          {{ t('knowledgeV3.graphLoading') }}
        </p>
      </div>
      <p v-else-if="graph && !visibleNodes.length" class="kg-status-pill pointer-events-none">
        {{ t('knowledgeV3.graphEmpty') }}
      </p>
    </div>
  </div>
</template>

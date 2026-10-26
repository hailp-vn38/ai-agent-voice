<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import cytoscape, { type Core } from 'cytoscape'
import { requestJson, withQuery } from '@/api/client'
import { formatApiError, isApiError } from '@/api/errors'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useTheme } from '@/composables/useTheme'

type GraphNode = { id: string; path: string; title: string; status: string; revision: number }
type GraphReference = { line: number; raw_target: string; alias?: string; anchor?: string }
type GraphEdge = { source_id: string; target_id: string; kind: string; lines: number[]; references: GraphReference[] }
type GraphUnresolved = { source_id: string; source_line: number; raw_target: string; reason: string }
type GraphSnapshot = { nodes: GraphNode[]; edges: GraphEdge[]; unresolved: GraphUnresolved[]; truncated: boolean }
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

const current = computed(() => graph.value?.nodes.find(node => node.id === selectedId.value) ?? null)
const selectedEdge = computed(() => {
  if (!selectedEdgeId.value) return null
  const index = Number(selectedEdgeId.value.replace('link_', ''))
  return graph.value?.edges[index] ?? null
})
const outgoing = computed(() => (graph.value?.edges ?? []).filter(e => e.source_id === selectedId.value))
const incoming = computed(() => (graph.value?.edges ?? []).filter(e => e.target_id === selectedId.value))
const unresolved = computed(() => (graph.value?.unresolved ?? []).filter(e => e.source_id === selectedId.value))
const names = computed(() => new Map((graph.value?.nodes ?? []).map(n => [n.id, n.title])))
function name(id: string): string { return names.value.get(id) ?? id }
function fitGraph() { cy?.fit() }
function relayout() { cy?.layout({ name: 'cose', animate: false, padding: 32 }).run() }
function focusEdge(edge: GraphEdge) {
  const index = graph.value?.edges.indexOf(edge) ?? -1
  if (index < 0) return
  selectedEdgeId.value = 'link_' + index
  cy?.getElementById(selectedEdgeId.value).select()
}
function focusNode(id: string) {
  selectedEdgeId.value = null
  selectedId.value = id
  cy?.getElementById(id).select()
}

function draw() {
  cy?.destroy()
  cy = null
  if (!canvas.value || !graph.value) return
  const snapshot = graph.value
  // Cytoscape canvas styles cannot resolve CSS vars directly. Use the matching
  // Studio semantic palette for each theme and redraw when ThemeToggle changes.
  const graphColors = dark.value
    ? { text: '#dad9e1', node: '#c5a6f3', warning: '#e5c078', selected: '#9edebc', edge: '#9896a8' }
    : { text: '#3e4255', node: '#7350b7', warning: '#a66a26', selected: '#268a62', edge: '#9599ae' }
  const elements = [
    ...snapshot.nodes.map(n => ({ data: { id: n.id, title: n.title, status: n.status } })),
    ...snapshot.edges.map((e, i) => ({
      data: { id: 'link_' + i, source: e.source_id, target: e.target_id, kind: e.kind },
    })),
  ]
  cy = cytoscape({
    container: canvas.value,
    elements,
    layout: { name: 'cose', animate: false, padding: 32, nodeRepulsion: () => 7000 },
    wheelSensitivity: 0.24,
    style: [
      { selector: 'node', style: {
        'label': 'data(title)', 'font-size': 11, 'text-wrap': 'wrap', 'text-max-width': '95px',
        'text-valign': 'bottom', 'text-margin-y': 7, 'color': graphColors.text,
        'background-color': graphColors.node, 'width': 20, 'height': 20,
      } },
      { selector: 'node[status = "draft"]', style: { 'background-color': graphColors.warning } },
      { selector: 'node:selected', style: { 'background-color': graphColors.selected, 'width': 27, 'height': 27 } },
      { selector: 'edge', style: {
        'width': 1.5, 'line-color': graphColors.edge, 'target-arrow-color': graphColors.edge,
        'target-arrow-shape': 'triangle', 'curve-style': 'bezier', 'opacity': 0.75,
      } },
    ],
  })
  cy.on('tap', 'node', event => {
    selectedEdgeId.value = null
    selectedId.value = event.target.id()
  })
  cy.on('tap', 'edge', event => { selectedEdgeId.value = event.target.id() })
  cy.on('mouseover', 'node', event => {
    const node = snapshot.nodes.find(n => n.id === event.target.id())
    hoverText.value = node ? node.title + ' · ' + node.path : ''
  })
  cy.on('mouseover', 'edge', event => {
    const edge = snapshot.edges[Number(event.target.id().replace('link_', ''))]
    hoverText.value = edge ? edge.source_id + ' → ' + edge.target_id : ''
  })
  cy.on('mouseout', 'node, edge', () => { hoverText.value = '' })
  if (selectedId.value) cy.getElementById(selectedId.value).select()
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
    if (!result.nodes.some(n => n.id === selectedId.value)) {
      selectedId.value = result.nodes.find(n => n.id === props.centerId)?.id ?? result.nodes[0]?.id ?? null
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
      <span v-if="graph?.truncated" class="text-xs text-amber-600">{{ t('knowledgeV3.graphTruncated') }}</span>
    </div>
    <p v-if="error" role="alert" class="rounded-lg border border-danger/30 p-3 text-sm text-danger">{{ error }}</p>
    <Button v-if="isStale" type="button" size="sm" variant="outline" @click="emit('rebuild')">
      {{ t('knowledgeV3.graphRebuild') }}
    </Button>
    <p v-if="scope === 'local' && !centerId" class="text-sm text-muted-foreground">{{ t('knowledgeV3.graphChooseWiki') }}</p>
    <div class="grid min-h-[410px] gap-3 lg:grid-cols-[minmax(0,1fr)_210px]">
      <div class="relative min-h-[410px] overflow-hidden rounded-lg border bg-muted/20">
        <div ref="canvas" role="img" :aria-label="t('knowledgeV3.graph')"
          class="absolute inset-0"></div>
        <p v-if="hoverText" class="pointer-events-none absolute bottom-3 left-3 max-w-[95%] rounded bg-background/95 p-2 text-xs shadow">
          {{ hoverText }}
        </p>
        <div v-if="loading" role="status" aria-busy="true" class="card-shimmer pointer-events-none absolute inset-0 bg-muted/15">
          <p class="absolute left-3 top-3 rounded bg-background/90 p-2 text-xs">
            {{ t('knowledgeV3.graphLoading') }}
          </p>
        </div>
        <p v-else-if="graph && !graph.nodes.length" class="pointer-events-none absolute left-3 top-3 text-xs">
          {{ t('knowledgeV3.graphEmpty') }}
        </p>
      </div>
      <aside class="max-h-[470px] space-y-3 overflow-auto rounded-lg border p-3 text-xs">
        <p class="font-semibold">{{ current?.title ?? t('knowledgeV3.graphChooseWiki') }}</p>
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
          <button v-for="node in graph?.nodes ?? []" :key="node.id" type="button"
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

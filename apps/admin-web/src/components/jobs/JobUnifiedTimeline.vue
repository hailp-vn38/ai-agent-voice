<script setup lang="ts">
import { Download, Eye } from '@lucide/vue'
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { agentJobsApi } from '@/api/agent-jobs'
import type { AdminJob, JobArtifact } from '@/api/types/agent-jobs'
import {
  buildExecutionTree,
  filterVisibleTreeNodes,
  treeNodeAncestors,
  treeNodeChildren,
  treeNodeDescendants,
  type ExecutionNodeState,
  type ExecutionNodeType,
  type ExecutionTreeNode,
} from '@/domain/jobs/execution-tree'
import { jobResultReport, type JobReport } from '@/domain/jobs/result-report'
import {
  extractMarkdownMediaEmbeds,
  mediaArtifactKind,
  type ActivityFilter,
  type UnifiedActivity,
} from '@/domain/jobs/unified-activity'
import { useI18n } from '@/composables/useI18n'
import ArtifactPreviewDialog, { artifactPreview } from './artifacts/ArtifactPreviewDialog.vue'

export interface JobCanvasStats {
  agentRunsCount: number | null
  llmRunsCount: number
  activeLlmCount: number
  pluginCallsCount: number
  tokenUsage: {
    tokens: number
    known: number
    total: number
  }
}

export interface CanvasMediaItem {
  id: string
  kind: 'image' | 'video'
  title: string
  subtitle: string | null
  src: string
  artifact?: JobArtifact
}

const props = defineProps<{
  job?: AdminJob | null
  jobId?: string
  activities: UnifiedActivity[]
  selectedId: string | null
  filter: ActivityFilter
  roles: string[]
  stats?: JobCanvasStats
}>()

const emit = defineEmits<{
  select: [id: string]
  filter: [value: ActivityFilter]
}>()

const { t } = useI18n()

const searchQuery = ref('')
const autoFollow = ref(true)
const detailOpenOnMobile = ref(false)
const copiedKey = ref<string | null>(null)
const treeScrollRef = ref<HTMLElement | null>(null)
const inspectorScrollRef = ref<HTMLElement | null>(null)

const treeNodes = computed<ExecutionTreeNode[]>(() => buildExecutionTree(props.activities, props.job))
const expanded = ref<Set<string>>(new Set(['root']))
const initializedExpandIds = new Set<string>()
const prevRunningNodeIds = new Set<string>()
let hasInitializedSelection = false

const selectedNodeId = ref<string | null>(null)

function findMatchingNode(targetId: string | null | undefined, nodes: ExecutionTreeNode[]): ExecutionTreeNode | null {
  if (!targetId) return null
  return (
    nodes.find(n => n.id === targetId || n.activityId === targetId) ??
    nodes.find(n => n.agentRunId && `agent:${n.agentRunId}` === targetId) ??
    null
  )
}

watch(
  treeNodes,
  nodes => {
    const nextExpanded = new Set(expanded.value)
    let expandChanged = false
    for (const node of nodes) {
      if ((node.type === 'root' || node.type === 'agent') && !initializedExpandIds.has(node.id)) {
        initializedExpandIds.add(node.id)
        nextExpanded.add(node.id)
        expandChanged = true
      }
    }
    if (expandChanged) {
      expanded.value = nextExpanded
    }

    const currentRunning = nodes.filter(n => n.state === 'running' && n.type !== 'root')
    const newlyRunningList = currentRunning.filter(n => !prevRunningNodeIds.has(n.id))
    const latestNewlyRunning = newlyRunningList.at(-1)
    const isInitial = !hasInitializedSelection
    hasInitializedSelection = true
    prevRunningNodeIds.clear()
    for (const r of currentRunning) prevRunningNodeIds.add(r.id)

    if (isInitial && props.selectedId) {
      const explicitInitial = findMatchingNode(props.selectedId, nodes)
      if (explicitInitial) {
        selectedNodeId.value = explicitInitial.id
        return
      }
    }

    if (autoFollow.value && latestNewlyRunning) {
      const nextExp = new Set(expanded.value)
      for (const anc of treeNodeAncestors(nodes, latestNewlyRunning.id)) {
        nextExp.add(anc.id)
      }
      expanded.value = nextExp
      selectedNodeId.value = latestNewlyRunning.id
      return
    }

    // Keep current selection if still valid
    if (selectedNodeId.value && nodes.some(n => n.id === selectedNodeId.value)) {
      return
    }

    // Match props.selectedId if provided
    const fromProp = findMatchingNode(props.selectedId, nodes)
    if (fromProp && fromProp.id !== 'root') {
      selectedNodeId.value = fromProp.id
      return
    }

    // Prefer Result node on completed job with output, or first active/child node, or root
    const resultNode = nodes.find(n => n.id === 'job:result')
    if (resultNode) {
      selectedNodeId.value = resultNode.id
      return
    }
    if (fromProp) {
      selectedNodeId.value = fromProp.id
      return
    }
    const firstNonRoot = nodes.find(n => n.id !== 'root')
    selectedNodeId.value = firstNonRoot?.id ?? nodes[0]?.id ?? null
  },
  { immediate: true },
)

watch(
  () => props.selectedId,
  nextId => {
    const matched = findMatchingNode(nextId, treeNodes.value)
    if (matched) {
      selectedNodeId.value = matched.id
    }
  },
)

const visibleNodes = computed(() =>
  filterVisibleTreeNodes(treeNodes.value, expanded.value, searchQuery.value),
)

const selectedNode = computed<ExecutionTreeNode | null>(() => {
  if (!treeNodes.value.length) return null
  return (
    treeNodes.value.find(n => n.id === selectedNodeId.value) ??
    visibleNodes.value[0] ??
    treeNodes.value[0] ??
    null
  )
})

const breadcrumbPath = computed<ExecutionTreeNode[]>(() => {
  const cur = selectedNode.value
  if (!cur) return []
  return [...treeNodeAncestors(treeNodes.value, cur.id), cur]
})

const parentOfSelected = computed<ExecutionTreeNode | null>(() => {
  const cur = selectedNode.value
  if (!cur || !cur.parent) return null
  return treeNodes.value.find(n => n.id === cur.parent) ?? null
})

const selectedDescendants = computed<ExecutionTreeNode[]>(() => {
  const cur = selectedNode.value
  if (!cur) return []
  return treeNodeDescendants(treeNodes.value, cur.id)
})

const selectedDirectChildren = computed<ExecutionTreeNode[]>(() => {
  const cur = selectedNode.value
  if (!cur) return []
  return treeNodeChildren(treeNodes.value, cur.id)
})

const selectedDescendantAgents = computed(
  () => selectedDescendants.value.filter(n => n.type === 'agent').length,
)
const selectedDescendantTools = computed(
  () => selectedDescendants.value.filter(n => n.type === 'tool').length,
)
const selectedDescendantLlms = computed(
  () => selectedDescendants.value.filter(n => n.type === 'llm').length,
)

function selectNode(id: string, follow = false) {
  const node = treeNodes.value.find(n => n.id === id)
  if (!node) return
  selectedNodeId.value = node.id
  if (follow) {
    const nextExp = new Set(expanded.value)
    for (const anc of treeNodeAncestors(treeNodes.value, node.id)) {
      nextExp.add(anc.id)
    }
    expanded.value = nextExp
    searchQuery.value = ''
  }
  detailOpenOnMobile.value = true
  emit('select', node.activityId ?? node.id)
  if (inspectorScrollRef.value && typeof inspectorScrollRef.value.scrollTo === 'function') {
    inspectorScrollRef.value.scrollTo({ top: 0, behavior: follow ? 'smooth' : 'instant' })
  }
}

function toggleNodeExpand(id: string) {
  const next = new Set(expanded.value)
  if (next.has(id)) next.delete(id)
  else next.add(id)
  expanded.value = next
}

function expandAll() {
  const next = new Set<string>()
  for (const n of treeNodes.value) {
    if (treeNodeChildren(treeNodes.value, n.id).length > 0) {
      next.add(n.id)
    }
  }
  expanded.value = next
}

function collapseAll() {
  expanded.value = new Set(['root'])
}

function stepSelection(delta: number) {
  const list = visibleNodes.value
  if (!list.length) return
  const idx = list.findIndex(x => x.id === selectedNode.value?.id)
  const targetIdx = idx === -1 ? 0 : Math.max(0, Math.min(list.length - 1, idx + delta))
  const next = list[targetIdx]
  if (next) selectNode(next.id, true)
}

// Top Stats Bar metrics
const rootNode = computed(() => treeNodes.value.find(n => n.id === 'root') ?? null)
const runtimeText = computed(() => rootNode.value?.duration ?? '—')
const llmCallsCount = computed(() =>
  props.stats ? props.stats.llmRunsCount : treeNodes.value.filter(n => n.type === 'llm').length,
)
const toolCallsCount = computed(() =>
  props.stats
    ? props.stats.pluginCallsCount
    : treeNodes.value.filter(n => n.type === 'tool' && n.activityKind === 'plugin').length,
)
const subagentsCount = computed(() => {
  const treeSubagents = treeNodes.value.filter(n => n.type === 'agent').length
  if (treeSubagents > 0) return treeSubagents
  if (props.stats?.agentRunsCount !== null && props.stats?.agentRunsCount !== undefined) {
    return Math.max(0, props.stats.agentRunsCount - 1)
  }
  return 0
})
const errorsCount = computed(
  () => treeNodes.value.filter(n => n.state === 'failed' && n.id !== 'root').length ||
    (rootNode.value?.state === 'failed' ? 1 : 0),
)
const totalTokensText = computed(() => {
  if (props.stats?.tokenUsage.known) {
    return props.stats.tokenUsage.tokens.toLocaleString()
  }
  const budgetTokens = props.job?.budget_used?.total_tokens
  if (typeof budgetTokens === 'number' && budgetTokens > 0) {
    return budgetTokens.toLocaleString()
  }
  return '—'
})
const agentRunsSummary = computed(() => {
  if (props.stats?.agentRunsCount !== null && props.stats?.agentRunsCount !== undefined) {
    return props.stats.agentRunsCount
      ? `1 root · ${Math.max(0, props.stats.agentRunsCount - 1)} sub-agents`
      : t('jobUi.noAgentRunsYet')
  }
  return `${subagentsCount.value}`
})

const runningNodesCount = computed(() => treeNodes.value.filter(n => n.state === 'running').length)

function iconType(type: ExecutionNodeType): string {
  switch (type) {
    case 'root':
      return 'bot'
    case 'agent':
      return 'agent'
    case 'tool':
      return 'terminal'
    case 'llm':
      return 'sparkles'
    case 'artifact':
    case 'result':
    case 'media':
      return 'file'
  }
}

function labelType(type: ExecutionNodeType): string {
  return t(('jobUi.nodeType.' + type) as 'jobUi.nodeType.root')
}

function labelStatus(state: ExecutionNodeState): string {
  return t(('jobUi.nodeState.' + state) as 'jobUi.nodeState.done')
}

function roleName(role: string | null): string {
  switch (role) {
    case 'orchestrator':
    case 'root':
      return t('jobUi.agentOrchestrator')
    case 'research':
    case 'wiki_curator':
      return t('jobUi.agentResearch')
    case 'document':
    case 'source_collector':
    case 'wiki_ingestor':
      return t('jobUi.agentDocument')
    case 'general':
      return t('jobUi.agentGeneral')
    default:
      return role || t('jobUi.agentLegacy')
  }
}

// Media & Artifact handling
const previewedArtifact = ref<JobArtifact | null>(null)
const downloadingArtifactId = ref<string | null>(null)
const mediaBlobUrls = ref<Record<string, string>>({})
let activeBlobUrls: Record<string, string> = {}
let loadedMediaJobId: string | undefined
let mediaLoadGeneration = 0

function revokeAllMediaBlobs() {
  for (const url of Object.values(activeBlobUrls)) {
    if (url.startsWith('blob:')) {
      URL.revokeObjectURL(url)
    }
  }
  activeBlobUrls = {}
  mediaBlobUrls.value = {}
}

watch(
  [() => props.jobId, () => props.activities],
  async ([jobId, activities]) => {
    const gen = ++mediaLoadGeneration
    if (jobId !== loadedMediaJobId) {
      revokeAllMediaBlobs()
      loadedMediaJobId = jobId
    }
    const visualArtifacts = new Map<string, JobArtifact>()
    for (const activity of activities) {
      for (const art of activity.artifacts ?? []) {
        if (mediaArtifactKind(art)) {
          visualArtifacts.set(art.id, art)
        }
      }
    }
    if (!jobId || visualArtifacts.size === 0 || typeof agentJobsApi.artifact !== 'function') {
      revokeAllMediaBlobs()
      return
    }
    for (const [id, url] of Object.entries(activeBlobUrls)) {
      if (!visualArtifacts.has(id)) {
        if (url.startsWith('blob:')) URL.revokeObjectURL(url)
        delete activeBlobUrls[id]
      }
    }
    mediaBlobUrls.value = { ...activeBlobUrls }
    await Promise.all(
      [...visualArtifacts.values()].map(async art => {
        if (activeBlobUrls[art.id]) return
        try {
          const file = await agentJobsApi.artifact(jobId, art.id)
          if (gen !== mediaLoadGeneration) return
          const mime = (art.media_type ?? art.mime_type ?? '').trim() || undefined
          const blobUrl = URL.createObjectURL(mime ? new Blob([file], { type: mime }) : file)
          activeBlobUrls = { ...activeBlobUrls, [art.id]: blobUrl }
          mediaBlobUrls.value = { ...activeBlobUrls }
        } catch {
          // Fallback to download_url or API path via resolveArtifactMediaSrc
        }
      }),
    )
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  mediaLoadGeneration++
  revokeAllMediaBlobs()
})

function resolveArtifactMediaSrc(art: JobArtifact): string {
  const blobUrl = mediaBlobUrls.value[art.id]
  if (blobUrl) return blobUrl
  if (art.download_url) return art.download_url
  if (props.jobId) {
    return `/api/admin/jobs/${encodeURIComponent(props.jobId)}/artifacts/${encodeURIComponent(art.id)}`
  }
  return ''
}

const selectedMediaItems = computed<CanvasMediaItem[]>(() => {
  const node = selectedNode.value
  if (!node) return []
  const items: CanvasMediaItem[] = []
  const seen = new Set<string>()
  const collectFromNode = (target: ExecutionTreeNode) => {
    for (const art of target.artifacts ?? []) {
      const kind = mediaArtifactKind(art)
      if (!kind || seen.has(`artifact:${art.id}`)) continue
      seen.add(`artifact:${art.id}`)
      const mime = art.media_type || art.mime_type || kind
      const sizeLabel = art.size_bytes >= 0 ? `${(art.size_bytes / 1024).toFixed(1)} KiB` : null
      items.push({
        id: `artifact:${art.id}`,
        kind,
        title: art.filename || art.name || art.id,
        subtitle: sizeLabel ? `${mime} · ${sizeLabel}` : mime,
        src: resolveArtifactMediaSrc(art),
        artifact: art,
      })
    }
    const embeds =
      target.mediaEmbeds && target.mediaEmbeds.length > 0
        ? target.mediaEmbeds
        : extractMarkdownMediaEmbeds(target.assistantMessage)
    for (const embed of embeds) {
      if (seen.has(embed.id)) continue
      seen.add(embed.id)
      items.push({
        id: embed.id,
        kind: embed.kind,
        title: embed.title,
        subtitle: embed.kind.toUpperCase(),
        src: embed.url,
      })
    }
  }
  collectFromNode(node)
  if (node.type === 'root') {
    const mediaNode = treeNodes.value.find(n => n.id === 'job:media')
    if (mediaNode) collectFromNode(mediaNode)
  }
  return items
})

const selectedArtifacts = computed<JobArtifact[]>(() => {
  const node = selectedNode.value
  if (!node) return []
  if (node.type === 'media') {
    return node.artifacts.filter(art => mediaArtifactKind(art) !== null)
  }
  const nonMedia = node.artifacts.filter(art => !mediaArtifactKind(art))
  if (nonMedia.length > 0) return nonMedia
  if (node.type === 'root') {
    const resultNode = treeNodes.value.find(n => n.id === 'job:result')
    if (resultNode) return resultNode.artifacts.filter(art => !mediaArtifactKind(art))
  }
  return []
})

const selectedReport = computed<JobReport>(() => {
  const node = selectedNode.value
  if (!node) return { kind: 'empty' }
  if (node.jobResult) return jobResultReport(node.jobResult)
  if (node.type === 'agent' && node.result && typeof node.result === 'object') {
    return jobResultReport(node.result as Record<string, unknown>)
  }
  return { kind: 'empty' }
})

const selectedAgentMessages = computed(() =>
  (selectedNode.value?.agentRun?.messages ?? []).filter(m => m.text && m.text.trim().length > 0),
)

const pendingLlmStage = computed(() => {
  const run = selectedNode.value?.llm
  if (!run || run.status !== 'running') return null
  switch (run.events.at(-1)?.event_type) {
    case 'llm_run_started':
      return t('jobUi.llmAcquiringProvider')
    case 'agent_run_started':
      return t('jobUi.llmWaitingResponse')
    case 'llm_provider_acquired':
      return t('jobUi.llmWaitingCapacity')
    case 'llm_request_started':
      return t('jobUi.llmWaitingResponse')
    case 'llm_response_received':
      return t('jobUi.llmValidating')
    default:
      return null
  }
})

function canPreviewArtifact(item: JobArtifact): boolean {
  const preview = artifactPreview(item)
  return Boolean(props.jobId) && preview.kind !== 'download' && item.size_bytes >= 0 && item.size_bytes <= preview.limit
}

async function downloadArtifact(item: JobArtifact) {
  if (!props.jobId || downloadingArtifactId.value) return
  downloadingArtifactId.value = item.id
  try {
    const data = await agentJobsApi.artifact(props.jobId, item.id)
    const url = URL.createObjectURL(data)
    const link = document.createElement('a')
    link.href = url
    link.download = item.filename || item.name || item.id
    link.click()
    window.setTimeout(() => URL.revokeObjectURL(url), 1000)
  } finally {
    downloadingArtifactId.value = null
  }
}

function formatJson(value: unknown): string {
  if (value === undefined || value === null) return '—'
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

async function copyJson(key: string, value: unknown) {
  try {
    await navigator.clipboard.writeText(formatJson(value))
    copiedKey.value = key
    window.setTimeout(() => {
      if (copiedKey.value === key) copiedKey.value = null
    }, 900)
  } catch {
    // Clipboard unavailable
  }
}

function hasObjectKeys(value: unknown): boolean {
  if (!value || typeof value !== 'object') return false
  return Object.keys(value as Record<string, unknown>).length > 0
}
</script>

<template>
  <section
    id="shell"
    class="studio-panel tree-shell min-w-0 overflow-hidden"
    :class="{ 'detail-open': detailOpenOnMobile }"
    :aria-label="t('jobUi.unifiedTimeline')"
  >
    <!-- Hidden SVG symbol sprite matching job-detail-hierarchy-compact.html -->
    <svg xmlns="http://www.w3.org/2000/svg" class="hidden" aria-hidden="true">
      <symbol id="i-bot" viewBox="0 0 24 24">
        <rect x="4" y="7" width="16" height="13" rx="3" />
        <path d="M12 3v4M9 3h6M8 12h.01M16 12h.01M9 16h6M2 12v3M22 12v3" />
      </symbol>
      <symbol id="i-agent" viewBox="0 0 24 24">
        <circle cx="12" cy="5" r="2.2" />
        <circle cx="5" cy="19" r="2.2" />
        <circle cx="19" cy="19" r="2.2" />
        <path d="M12 7.5v5M12 12.5 5 16.5M12 12.5l7 4" />
      </symbol>
      <symbol id="i-terminal" viewBox="0 0 24 24">
        <rect x="3" y="4" width="18" height="16" rx="2" />
        <path d="m7 9 3 3-3 3M13 16h5" />
      </symbol>
      <symbol id="i-sparkles" viewBox="0 0 24 24">
        <path d="m12 3 1.7 5.3L19 10l-5.3 1.7L12 17l-1.7-5.3L5 10l5.3-1.7L12 3ZM19 16l.6 1.4L21 18l-1.4.6L19 20l-.6-1.4L17 18l1.4-.6L19 16ZM5 2l.6 1.4L7 4l-1.4.6L5 6l-.6-1.4L3 4l1.4-.6L5 2Z" />
      </symbol>
      <symbol id="i-file" viewBox="0 0 24 24">
        <path d="M7 3h7l5 5v13H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Z" />
        <path d="M14 3v6h5M9 14h6M9 17h7" />
      </symbol>
      <symbol id="i-chevron" viewBox="0 0 24 24">
        <path d="m9 18 6-6-6-6" />
      </symbol>
      <symbol id="i-down" viewBox="0 0 24 24">
        <path d="m6 9 6 6 6-6" />
      </symbol>
      <symbol id="i-search" viewBox="0 0 24 24">
        <circle cx="10.5" cy="10.5" r="6.5" />
        <path d="m16 16 5 5" />
      </symbol>
      <symbol id="i-clock" viewBox="0 0 24 24">
        <circle cx="12" cy="12" r="9" />
        <path d="M12 7v5l4 2" />
      </symbol>
      <symbol id="i-coins" viewBox="0 0 24 24">
        <ellipse cx="12" cy="6" rx="8" ry="3" />
        <path d="M4 6v6c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6" />
      </symbol>
      <symbol id="i-copy" viewBox="0 0 24 24">
        <rect x="8" y="8" width="12" height="12" rx="2" />
        <path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" />
      </symbol>
      <symbol id="i-check" viewBox="0 0 24 24">
        <path d="m4 12 5 5L20 6" />
      </symbol>
      <symbol id="i-alert" viewBox="0 0 24 24">
        <path d="m12 3 10 18H2L12 3Z" />
        <path d="M12 9v5M12 18h.01" />
      </symbol>
      <symbol id="i-layer" viewBox="0 0 24 24">
        <path d="m12 3 9 5-9 5-9-5 9-5ZM3 12l9 5 9-5M3 16l9 5 9-5" />
      </symbol>
      <symbol id="i-arrow-left" viewBox="0 0 24 24">
        <path d="m15 18-6-6 6-6M9 12h12" />
      </symbol>
      <symbol id="i-pin" viewBox="0 0 24 24">
        <path d="M8 3h8l-1 7 3 3v2H6v-2l3-3-1-7ZM12 15v7" />
      </symbol>
    </svg>

    <!-- Stats Bar -->
    <div class="stats" data-canvas-stats>
      <div class="stat" data-canvas-stat="runtime">
        <svg class="icon"><use href="#i-clock" /></svg>
        <span class="label">{{ t('jobUi.statRuntime') }}</span>
        <strong id="duration">{{ runtimeText }}</strong>
      </div>
      <div class="stat" data-canvas-stat="total-tokens">
        <svg class="icon"><use href="#i-coins" /></svg>
        <span class="label">{{ t('jobUi.statTokens') }}</span>
        <strong id="cost">{{ totalTokensText }}</strong>
      </div>
      <div class="stat" data-canvas-stat="llm-runs">
        <span class="label">{{ t('jobUi.statLlmCalls') }}</span>
        <strong id="llmCount">{{ llmCallsCount }}</strong>
      </div>
      <div class="stat" data-canvas-stat="plugin-calls">
        <span class="label">{{ t('jobUi.statToolCalls') }}</span>
        <strong id="toolCount">{{ toolCallsCount }}</strong>
      </div>
      <div class="stat" data-canvas-stat="agent-runs" :title="agentRunsSummary">
        <span class="label">{{ t('jobUi.statSubagents') }}</span>
        <strong id="agentCount">{{ subagentsCount }}</strong>
        <span v-if="stats?.agentRunsCount !== null && stats?.agentRunsCount !== undefined" class="sr-only">
          {{ agentRunsSummary }}
        </span>
      </div>
      <div class="stat" data-canvas-stat="errors">
        <span class="label">{{ t('jobUi.statErrors') }}</span>
        <strong id="errorCount" :class="{ danger: errorsCount > 0 }">{{ errorsCount }}</strong>
      </div>
    </div>

    <!-- Split Workspace: Execution Tree + Inline Inspector -->
    <div class="workspace">
      <aside class="treepane">
        <div class="panehead">
          <svg class="icon text-studio-violet"><use href="#i-agent" /></svg>
          <h2>{{ t('jobUi.executionTree') }}</h2>
          <span id="nodeCount" class="badge" data-tree-node-count>{{ treeNodes.length }}</span>
          <span class="grow" />
          <button
            id="collapseBtn"
            type="button"
            class="small-btn"
            data-tree-collapse-all
            :title="t('jobUi.collapseAllAgents')"
            :aria-label="t('jobUi.collapseAllAgents')"
            @click="collapseAll"
          >
            <svg class="icon sm"><use href="#i-layer" /></svg>
          </button>
          <button
            id="expandBtn"
            type="button"
            class="small-btn"
            data-tree-expand-all
            :title="t('jobUi.expandAllAgents')"
            :aria-label="t('jobUi.expandAllAgents')"
            @click="expandAll"
          >
            <svg class="icon sm"><use href="#i-down" /></svg>
          </button>
        </div>

        <label class="search">
          <svg class="icon sm"><use href="#i-search" /></svg>
          <input
            id="searchInput"
            v-model="searchQuery"
            type="search"
            data-tree-search
            :placeholder="t('jobUi.searchTreePlaceholder')"
          >
        </label>

        <div
          v-if="roles.length > 0 || filter.kind !== 'all'"
          class="filterbar"
          role="group"
          :aria-label="t('jobUi.timelineFilter')"
        >
          <button
            type="button"
            class="filter"
            :class="{ active: filter.kind === 'all' }"
            :aria-pressed="filter.kind === 'all'"
            @click="emit('filter', { kind: 'all' })"
          >
            {{ t('jobUi.timelineAll') }}
          </button>
          <button
            type="button"
            class="filter"
            :class="{ active: filter.kind === 'llm' }"
            :aria-pressed="filter.kind === 'llm'"
            @click="emit('filter', { kind: 'llm' })"
          >
            {{ t('jobUi.filterLlm') }}
          </button>
          <button
            type="button"
            class="filter"
            :class="{ active: filter.kind === 'plugin' }"
            :aria-pressed="filter.kind === 'plugin'"
            @click="emit('filter', { kind: 'plugin' })"
          >
            {{ t('jobUi.filterPlugin') }}
          </button>
          <button
            type="button"
            class="filter"
            :class="{ active: filter.kind === 'root' }"
            :aria-pressed="filter.kind === 'root'"
            @click="emit('filter', { kind: 'root' })"
          >
            {{ t('jobUi.filterRoot') }}
          </button>
          <button
            v-for="role in roles"
            :key="role"
            type="button"
            class="filter"
            :class="{ active: filter.kind === 'role' && filter.role === role }"
            :aria-pressed="filter.kind === 'role' && filter.role === role"
            @click="emit('filter', { kind: 'role', role })"
          >
            {{ roleName(role) }}
          </button>
        </div>

        <div id="treeScroll" ref="treeScrollRef" class="treescroll">
          <div id="tree" role="tree" :aria-label="t('jobUi.executionTree')">
            <template v-if="visibleNodes.length">
              <div
                v-for="n in visibleNodes"
                :key="n.id"
                class="tree-row"
                :class="[
                  treeNodeAncestors(treeNodes, n.id).length === 0 ? 'depth0' : 'depth1',
                  {
                    selected: selectedNode?.id === n.id,
                    running: n.state === 'running',
                  },
                ]"
                :style="{ '--depth': treeNodeAncestors(treeNodes, n.id).length }"
                :data-node="n.id"
                :data-step="n.activityId || n.id"
                :data-kind="n.activityKind || n.type"
                :data-agent-run-id="n.type === 'agent' || n.type === 'root' ? (n.agentRunId ?? undefined) : undefined"
                role="treeitem"
                tabindex="0"
                :aria-selected="selectedNode?.id === n.id"
                :title="`${n.name} · ${labelStatus(n.state)}`"
                @click="selectNode(n.id, false)"
                @keydown.enter.prevent="selectNode(n.id, false)"
                @keydown.space.prevent="selectNode(n.id, false)"
              >
                <span
                  v-if="treeNodeAncestors(treeNodes, n.id).length > 0"
                  class="branch-elbow"
                  aria-hidden="true"
                />
                <span class="disclosure">
                  <button
                    v-if="treeNodeChildren(treeNodes, n.id).length > 0"
                    type="button"
                    :aria-label="`${expanded.has(n.id) ? 'Collapse' : 'Expand'} ${n.name}`"
                    :data-collapse="n.id"
                    @click.stop="toggleNodeExpand(n.id)"
                  >
                    <svg class="icon sm">
                      <use :href="expanded.has(n.id) ? '#i-down' : '#i-chevron'" />
                    </svg>
                  </button>
                </span>
                <span class="nodeicon" :class="n.type">
                  <svg class="icon sm"><use :href="`#i-${iconType(n.type)}`" /></svg>
                </span>
                <span
                  class="title"
                  :class="{
                    rootlabel: n.type === 'root',
                    'sub-label': n.type === 'agent',
                  }"
                >
                  {{ n.name }}
                </span>
                <span
                  v-if="n.type === 'agent' && treeNodeChildren(treeNodes, n.id).length > 0"
                  class="sub-count"
                >
                  {{ treeNodeChildren(treeNodes, n.id).length }}
                </span>
                <span class="time">{{ n.duration }}</span>
                <span class="state-dot" :class="n.state" />
              </div>
            </template>
            <div v-else class="empty-search">
              {{ searchQuery.trim() ? t('jobUi.emptyTreeSearch') : t('jobUi.noTimelineActivity') }}
            </div>
          </div>
        </div>

        <div class="treefooter">
          <span class="live-dot" :class="{ active: runningNodesCount > 0 }" />
          <span id="footerState">
            {{
              runningNodesCount > 0
                ? `${runningNodesCount} ${t('jobUi.nodesRunning')}`
                : t('jobUi.treeIdle')
            }}
          </span>
          <button
            id="followBtn"
            type="button"
            class="small-btn"
            :class="{ on: autoFollow }"
            data-follow-btn
            :title="t('jobUi.autoFollowTitle')"
            @click="autoFollow = !autoFollow"
          >
            <svg class="icon sm"><use href="#i-pin" /></svg>
            <span id="followText">{{ autoFollow ? t('jobUi.followOn') : t('jobUi.followOff') }}</span>
          </button>
        </div>
      </aside>

      <!-- Right Inline Inspector -->
      <main class="inspector" data-tree-inspector :data-inspected-node="selectedNode?.id">
        <div class="inspector-top">
          <button
            id="backBtn"
            type="button"
            class="small-btn mobile-only"
            :title="t('jobUi.backToTree')"
            :aria-label="t('jobUi.backToTree')"
            @click="detailOpenOnMobile = false"
          >
            <svg class="icon"><use href="#i-arrow-left" /></svg>
          </button>
          <div id="breadcrumb" class="breadcrumb">
            <template v-for="(crumb, idx) in breadcrumbPath" :key="crumb.id">
              <span v-if="idx > 0" class="sep">/</span>
              <button
                type="button"
                class="crumb-item"
                :class="{ current: idx === breadcrumbPath.length - 1 }"
                @click="selectNode(crumb.id, false)"
              >
                {{ crumb.name }}
              </button>
            </template>
          </div>
          <span class="grow" />
          <button
            id="prevBtn"
            type="button"
            class="small-btn"
            data-prev-node
            :title="t('jobUi.prevNode')"
            :aria-label="t('jobUi.prevNode')"
            @click="stepSelection(-1)"
          >
            <svg class="icon sm" style="transform: rotate(180deg)"><use href="#i-chevron" /></svg>
          </button>
          <button
            id="nextBtn"
            type="button"
            class="small-btn"
            data-next-node
            :title="t('jobUi.nextNode')"
            :aria-label="t('jobUi.nextNode')"
            @click="stepSelection(1)"
          >
            <svg class="icon sm"><use href="#i-chevron" /></svg>
          </button>
        </div>

        <div id="inspectorScroll" ref="inspectorScrollRef" class="inspector-scroll">
          <div v-if="selectedNode" id="detail" class="inspector-inner" :data-inspected-node="selectedNode.id">
            <div class="detail-heading">
              <div class="detail-avatar" :class="selectedNode.type">
                <svg class="icon lg"><use :href="`#i-${iconType(selectedNode.type)}`" /></svg>
              </div>
              <div class="min-w-0 flex-1">
                <h2>{{ selectedNode.name }}</h2>
                <div class="metadata">
                  <span
                    :class="
                      selectedNode.state === 'done'
                        ? 'good'
                        : selectedNode.state === 'running'
                          ? 'run'
                          : selectedNode.state === 'failed'
                            ? 'bad'
                            : ''
                    "
                  >
                    ● {{ labelStatus(selectedNode.state) }}
                  </span>
                  <span class="point">·</span>
                  <span class="capitalize">{{ selectedNode.rawStatus }}</span>
                  <span class="point">·</span>
                  <span>{{ selectedNode.duration }}</span>
                  <span class="point">·</span>
                  <span>{{ labelType(selectedNode.type) }}</span>
                </div>
              </div>
            </div>

            <p class="summary">{{ selectedNode.desc }}</p>

            <p
              v-if="selectedNode.step?.attempt_state === 'unknown_outcome'"
              role="alert"
              class="unknown-alert"
            >
              {{ t('jobUi.unknownOutcomeWarning') }}
            </p>

            <!-- Context Line -->
            <div v-if="selectedNode.type === 'root' || selectedNode.type === 'agent'" class="context-line">
              <svg class="icon sm"><use href="#i-agent" /></svg>
              <span>{{ t('jobUi.childAgents') }}</span>
              <strong>{{ selectedDescendantAgents }}</strong>
              <span class="point">·</span>
              <span>{{ t('jobUi.statToolCalls') }}</span>
              <strong>{{ selectedDescendantTools }}</strong>
              <span class="point">·</span>
              <span>{{ t('jobUi.statLlmCalls') }}</span>
              <strong>{{ selectedDescendantLlms }}</strong>
              <span v-if="selectedNode.agentRunId" class="tag font-mono">{{ selectedNode.agentRunId }}</span>
              <span v-if="selectedNode.state === 'running'" class="tag live">{{ t('jobUi.liveExecution') }}</span>
            </div>
            <div v-else class="context-line">
              <svg class="icon sm"><use :href="selectedNode.type === 'llm' ? '#i-sparkles' : '#i-terminal'" /></svg>
              <span>{{ t('jobUi.executor') }}</span>
              <strong>{{ parentOfSelected?.name || selectedNode.role || 'Agent' }}</strong>
              <span class="point">·</span>
              <strong>
                {{
                  selectedNode.type === 'llm'
                    ? t('jobUi.modelRequest')
                    : selectedNode.type === 'tool'
                      ? t('jobUi.toolInvocation')
                      : labelType(selectedNode.type)
                }}
              </strong>
              <span v-if="selectedNode.step?.plugin_id" class="tag">
                {{ selectedNode.step.plugin_id }}<template v-if="selectedNode.step.operation"> · {{ selectedNode.step.operation }}</template>
              </span>
              <span v-if="selectedNode.step?.step_key" class="tag font-mono">
                {{ selectedNode.step.step_key }}<template v-if="selectedNode.step.status"> · {{ selectedNode.step.status }}</template><template v-if="selectedNode.step.attempt_state"> · {{ selectedNode.step.attempt_state }}</template>
              </span>
              <span v-if="selectedNode.llm" class="tag">
                {{ selectedNode.llm.provider }}<template v-if="selectedNode.llm.model"> · {{ selectedNode.llm.model }}</template>
              </span>
              <span
                v-if="selectedNode.llm && (selectedNode.llm.inputTokens !== null || selectedNode.llm.outputTokens !== null)"
                class="tag font-mono"
              >
                {{ selectedNode.llm.inputTokens ?? '—' }} / {{ selectedNode.llm.outputTokens ?? '—' }} tokens
              </span>
            </div>

            <!-- Agent Message (when distinct from summary) -->
            <template v-if="selectedNode.assistantMessage && selectedNode.assistantMessage !== selectedNode.desc">
              <div class="section-title">
                <svg class="icon sm"><use href="#i-sparkles" /></svg>
                {{ t('jobUi.agentMessage') }}
              </div>
              <div class="message-box">{{ selectedNode.assistantMessage }}</div>
            </template>

            <!-- Tool Input / Arguments -->
            <template v-if="selectedNode.args !== null && selectedNode.args !== undefined && hasObjectKeys(selectedNode.args)">
              <div class="section-title">
                <svg class="icon sm"><use :href="selectedNode.type === 'llm' ? '#i-sparkles' : '#i-terminal'" /></svg>
                {{ selectedNode.type === 'llm' ? t('jobUi.modelRequest') : t('jobUi.toolArguments') }}
              </div>
              <div class="codebox" data-codebox="arguments">
                <div class="codehead">
                  <svg class="icon sm"><use href="#i-terminal" /></svg>
                  arguments · JSON
                  <button
                    type="button"
                    data-copy="arguments"
                    :title="t('jobUi.copyContent')"
                    @click="copyJson('arguments', selectedNode.args)"
                  >
                    <svg class="icon sm"><use :href="copiedKey === 'arguments' ? '#i-check' : '#i-copy'" /></svg>
                  </button>
                </div>
                <pre>{{ formatJson(selectedNode.args) }}</pre>
              </div>
            </template>

            <!-- Result (Markdown report and/or structured JSON) -->
            <template v-if="selectedReport.kind !== 'empty' || (selectedNode.result !== null && selectedNode.result !== undefined)">
              <div class="section-title">
                <svg class="icon sm"><use href="#i-check" /></svg>
                {{ t('jobUi.result') }}
                <span class="muted">
                  {{ selectedNode.state === 'failed' ? t('jobUi.blockedOutput') : t('jobUi.structuredOutput') }}
                </span>
              </div>
              <div v-if="selectedReport.kind === 'markdown'" class="markdown-box">
                <article
                  class="result-markdown"
                  data-result-markdown
                  v-html="(selectedReport as { kind: 'markdown'; html: string }).html"
                />
              </div>
              <div
                v-if="selectedNode.result !== null && selectedNode.result !== undefined && (selectedReport.kind !== 'markdown' || selectedNode.type === 'tool')"
                class="codebox"
                data-codebox="result"
              >
                <div class="codehead">
                  <svg class="icon sm"><use href="#i-terminal" /></svg>
                  response · JSON
                  <button
                    type="button"
                    data-copy="result"
                    :title="t('jobUi.copyContent')"
                    @click="copyJson('result', selectedNode.result)"
                  >
                    <svg class="icon sm"><use :href="copiedKey === 'result' ? '#i-check' : '#i-copy'" /></svg>
                  </button>
                </div>
                <pre>{{ formatJson(selectedNode.result) }}</pre>
              </div>
            </template>

            <!-- Visual Media (Images & Videos) -->
            <template v-if="selectedMediaItems.length > 0">
              <div class="section-title">
                <svg class="icon sm"><use href="#i-file" /></svg>
                {{ t('jobUi.media') }}
                <span class="muted">{{ selectedMediaItems.length }}</span>
              </div>
              <div class="media-list" data-modal-media-list>
                <div
                  v-for="media in selectedMediaItems"
                  :key="media.id"
                  class="media-item-card"
                  :data-media-kind="media.kind"
                  :data-modal-media-kind="media.kind"
                >
                  <div class="media-item-preview">
                    <video
                      v-if="media.kind === 'video' && media.src"
                      :src="media.src"
                      controls
                      preload="metadata"
                      class="media-card-video"
                      data-canvas-media-video
                      data-modal-media-video
                    />
                    <img
                      v-else-if="media.kind === 'image' && media.src"
                      :src="media.src"
                      :alt="media.title"
                      loading="lazy"
                      class="media-card-image"
                      data-canvas-media-image
                      data-modal-media-image
                    >
                  </div>
                  <div class="media-item-meta">
                    <div class="min-w-0">
                      <strong class="block truncate">{{ media.title }}</strong>
                      <small v-if="media.subtitle" class="block truncate text-muted-foreground">{{ media.subtitle }}</small>
                    </div>
                    <div v-if="media.artifact && jobId" class="flex shrink-0 items-center gap-1.5">
                      <button
                        type="button"
                        class="small-btn text"
                        :disabled="downloadingArtifactId !== null"
                        @click.stop="downloadArtifact(media.artifact)"
                      >
                        <Download class="size-3" />{{ t('jobUi.download') }}
                      </button>
                      <button
                        v-if="canPreviewArtifact(media.artifact)"
                        type="button"
                        class="small-btn text"
                        data-artifact-preview
                        data-canvas-artifact-preview
                        @click.stop="previewedArtifact = media.artifact"
                      >
                        <Eye class="size-3" />Preview
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            </template>

            <!-- Artifacts -->
            <template v-if="selectedArtifacts.length > 0">
              <div class="section-title">
                <svg class="icon sm"><use href="#i-file" /></svg>
                {{ t('jobUi.artifacts') }}
                <span class="muted">{{ selectedArtifacts.length }}</span>
              </div>
              <div
                v-for="art in selectedArtifacts"
                :key="art.id"
                class="artifact-row"
              >
                <svg class="icon sm"><use href="#i-file" /></svg>
                <strong class="truncate" :title="art.filename || art.name || art.id">
                  {{ art.filename || art.name || art.id }}
                </strong>
                <small class="truncate">
                  {{ art.media_type || art.mime_type }}<template v-if="art.size_bytes >= 0"> · {{ (art.size_bytes / 1024).toFixed(1) }} KiB</template>
                </small>
                <div v-if="jobId" class="flex shrink-0 items-center gap-1.5">
                  <button
                    type="button"
                    class="small-btn text"
                    :disabled="downloadingArtifactId !== null"
                    @click.stop="downloadArtifact(art)"
                  >
                    <Download class="size-3" />{{ t('jobUi.download') }}
                  </button>
                  <button
                    v-if="canPreviewArtifact(art)"
                    type="button"
                    class="small-btn text"
                    data-artifact-preview
                    data-canvas-artifact-preview
                    @click.stop="previewedArtifact = art"
                  >
                    <Eye class="size-3" />Preview
                  </button>
                </div>
              </div>
            </template>

            <!-- Agent Conversation Messages -->
            <template v-if="selectedAgentMessages.length > 0">
              <div class="section-title">
                <svg class="icon sm"><use href="#i-sparkles" /></svg>
                {{ t('jobUi.agentMessages') }}
                <span class="muted">{{ selectedAgentMessages.length }}</span>
              </div>
              <div class="space-y-1.5">
                <div
                  v-for="msg in selectedAgentMessages"
                  :key="msg.id"
                  class="message-box"
                >
                  <div class="mb-1 flex items-center justify-between text-[10px] font-semibold uppercase text-muted-foreground">
                    <span>{{ msg.role }}</span>
                    <span class="font-mono">#{{ msg.sequence }}</span>
                  </div>
                  <p class="m-0 whitespace-pre-wrap break-words">{{ msg.text }}</p>
                </div>
              </div>
            </template>

            <!-- Execution Events Feed -->
            <div class="section-title">
              <svg class="icon sm"><use href="#i-clock" /></svg>
              {{ t('jobUi.executionEvents') }}
              <span class="muted">{{ selectedNode.events.length }} {{ t('jobUi.entriesCount') }}</span>
            </div>
            <div class="feed">
              <div
                v-for="(ev, idx) in selectedNode.events"
                :key="idx"
                class="event"
                :class="ev.kind"
              >
                <div class="timestamp">{{ ev.time }}</div>
                <div class="eventbody">
                  <div class="eventname">
                    <svg class="icon sm">
                      <use
                        :href="
                          ev.kind === 'error'
                            ? '#i-alert'
                            : ev.kind === 'output'
                              ? '#i-check'
                              : ev.kind === 'processing'
                                ? '#i-sparkles'
                                : '#i-terminal'
                        "
                      />
                    </svg>
                    <span>{{ ev.title }}</span>
                    <span v-if="ev.validationPath" class="ml-1 break-all font-mono text-danger">
                      {{ ev.validationPath }}
                    </span>
                  </div>
                  <div class="message">{{ ev.detail }}</div>
                </div>
              </div>
              <div v-if="pendingLlmStage" class="event processing">
                <div class="timestamp">live</div>
                <div class="eventbody">
                  <div class="eventname">
                    <svg class="icon sm"><use href="#i-sparkles" /></svg>
                    <span>{{ pendingLlmStage }}</span>
                  </div>
                </div>
              </div>
            </div>
            <p v-if="selectedNode.errorCode" class="error-code-box">
              {{ selectedNode.errorCode }}
            </p>

            <!-- Child Execution List -->
            <template
              v-if="
                (selectedNode.type === 'root' || selectedNode.type === 'agent') &&
                selectedDirectChildren.length > 0
              "
            >
              <div class="section-title">
                <svg class="icon sm"><use href="#i-layer" /></svg>
                {{ t('jobUi.childExecution') }}
                <span class="muted">{{ selectedDirectChildren.length }} {{ t('jobUi.directChildren') }}</span>
              </div>
              <button
                v-for="child in selectedDirectChildren"
                :key="child.id"
                type="button"
                class="artifact-row child-btn"
                :data-child="child.id"
                @click="selectNode(child.id, false)"
              >
                <span class="nodeicon" :class="child.type">
                  <svg class="icon sm"><use :href="`#i-${iconType(child.type)}`" /></svg>
                </span>
                <strong class="truncate">{{ child.name }}</strong>
                <span v-if="child.desc && child.desc !== child.name" class="truncate text-[10.5px] text-muted-foreground">
                  {{ child.desc }}
                </span>
                <small>{{ labelStatus(child.state) }}</small>
                <svg class="icon sm"><use href="#i-chevron" /></svg>
              </button>
            </template>
          </div>
          <p v-else class="p-6 text-sm text-muted-foreground">
            {{ t('jobUi.noTimelineActivity') }}
          </p>
        </div>
      </main>
    </div>

    <ArtifactPreviewDialog
      v-if="previewedArtifact && jobId"
      :key="previewedArtifact.id"
      :job-id="jobId"
      :artifact="previewedArtifact"
      @close="previewedArtifact = null"
      @download="previewedArtifact && downloadArtifact(previewedArtifact)"
    />
  </section>
</template>

<style scoped>
.tree-shell {
  font-size: 12px;
  color: var(--foreground);
  background: var(--card);
}
.icon {
  width: 15px;
  height: 15px;
  stroke: currentColor;
  stroke-width: 1.8;
  fill: none;
  stroke-linecap: round;
  stroke-linejoin: round;
  flex: none;
  display: block;
}
.icon.sm {
  width: 13px;
  height: 13px;
}
.icon.lg {
  width: 17px;
  height: 17px;
}
.grow {
  flex: 1;
  min-width: 0;
}
.small-btn {
  border: 1px solid transparent;
  border-radius: 6px;
  min-width: 27px;
  height: 27px;
  background: transparent;
  color: var(--muted-foreground);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  padding: 0 6px;
  cursor: pointer;
  transition: background 0.12s, color 0.12s, border-color 0.12s;
}
.small-btn:hover:not(:disabled) {
  background: var(--muted);
  color: var(--foreground);
}
.small-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.small-btn.text {
  padding: 0 9px;
  border-color: var(--border);
  background: color-mix(in oklab, var(--surface) 75%, var(--card));
  font-size: 11px;
  gap: 5px;
  color: var(--foreground);
}
.small-btn.on {
  color: var(--primary);
  background: color-mix(in oklab, var(--primary) 14%, transparent);
  border-color: color-mix(in oklab, var(--primary) 38%, var(--border));
}

/* Stats Bar */
.stats {
  min-height: 49px;
  display: flex;
  align-items: center;
  padding: 0 17px;
  gap: 0;
  border-bottom: 1px solid var(--border);
  background: color-mix(in oklab, var(--surface) 65%, var(--card));
  overflow-x: auto;
}
.stat {
  display: flex;
  align-items: center;
  gap: 7px;
  padding-right: 22px;
  margin-right: 22px;
  border-right: 1px solid var(--border);
  min-width: 0;
  white-space: nowrap;
}
.stat:last-child {
  border: 0;
  margin: 0;
}
.stat .label {
  color: var(--muted-foreground);
  font-size: 11px;
}
.stat strong {
  font-size: 12px;
  color: var(--foreground);
  font-weight: 650;
  font-variant-numeric: tabular-nums;
}
.stat strong.danger {
  color: var(--danger);
}
.stat svg {
  width: 13px;
  height: 13px;
  color: var(--muted-foreground);
}

/* Split Workspace */
.workspace {
  height: clamp(540px, calc(100dvh - 230px), 820px);
  display: grid;
  grid-template-columns: 324px minmax(0, 1fr);
  grid-template-rows: minmax(0, 1fr);
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}
.treepane {
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--border);
  min-width: 0;
  min-height: 0;
  height: 100%;
  overflow: hidden;
  background: color-mix(in oklab, var(--surface) 45%, var(--card));
}
.panehead {
  height: 42px;
  min-height: 42px;
  flex: none;
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 0 12px;
  border-bottom: 1px solid var(--border);
}
.panehead h2 {
  font-size: 12px;
  font-weight: 650;
  margin: 0;
  color: var(--foreground);
}
.badge {
  color: var(--muted-foreground);
  border: 1px solid var(--border);
  background: var(--muted);
  padding: 1px 6px;
  border-radius: 4px;
  font-size: 10px;
  font-variant-numeric: tabular-nums;
}
.panehead .small-btn {
  width: 25px;
  height: 25px;
  min-width: 25px;
}
.search {
  display: flex;
  align-items: center;
  gap: 7px;
  color: var(--muted-foreground);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 0 8px;
  margin: 9px 11px 6px;
  height: 29px;
  flex: none;
  background: var(--background);
}
.search:focus-within {
  border-color: var(--ring);
  box-shadow: 0 0 0 2px color-mix(in oklab, var(--ring) 22%, transparent);
}
.search input {
  background: none;
  border: 0;
  outline: 0;
  color: var(--foreground);
  min-width: 0;
  width: 100%;
  font-size: 11px;
}
.search input::placeholder {
  color: var(--muted-foreground);
  opacity: 0.8;
}
.filterbar {
  display: flex;
  flex-wrap: wrap;
  flex: none;
  gap: 4px;
  padding: 0 11px 6px;
}
.filter {
  padding: 2px 7px;
  border-radius: 5px;
  background: var(--background);
  border: 1px solid var(--border);
  color: var(--muted-foreground);
  font-size: 10px;
  cursor: pointer;
  transition: border-color 0.12s, background 0.12s, color 0.12s;
}
.filter:hover {
  color: var(--foreground);
  border-color: var(--ring);
}
.filter.active {
  border-color: color-mix(in oklab, var(--primary) 50%, var(--border));
  background: color-mix(in oklab, var(--accent) 75%, transparent);
  color: var(--accent-foreground);
  font-weight: 600;
}

/* Tree Rows */
.treescroll {
  flex: 1 1 0%;
  overflow-y: auto;
  overflow-x: auto;
  min-height: 0;
  padding: 3px 7px 28px 7px;
  scrollbar-width: thin;
  scroll-behavior: smooth;
  overscroll-behavior: contain;
}
.tree-row {
  --depth: 0;
  position: relative;
  display: flex;
  align-items: center;
  gap: 5px;
  height: 31px;
  width: 100%;
  padding: 0 7px 0 calc(5px + var(--depth) * 18px);
  border: 1px solid transparent;
  border-radius: 5px;
  background: transparent;
  text-align: left;
  color: color-mix(in oklab, var(--foreground) 86%, var(--muted-foreground));
  cursor: pointer;
  transition: background 0.1s, border-color 0.1s, color 0.1s;
}
.tree-row:hover {
  background: color-mix(in oklab, var(--muted) 70%, transparent);
  color: var(--foreground);
}
.tree-row.selected {
  background: color-mix(in oklab, var(--accent) 75%, transparent);
  border-color: color-mix(in oklab, var(--primary) 45%, var(--border));
  color: var(--accent-foreground);
}
.tree-row.running:not(.selected) {
  background: color-mix(in oklab, var(--studio-cyan) 12%, transparent);
}
.tree-row .disclosure {
  width: 13px;
  height: 22px;
  display: grid;
  place-items: center;
  flex: none;
  color: var(--muted-foreground);
}
.tree-row .disclosure button {
  background: none;
  border: 0;
  display: grid;
  place-items: center;
  padding: 0;
  color: inherit;
  width: 15px;
  height: 22px;
  cursor: pointer;
}
.tree-row .disclosure button:hover {
  color: var(--foreground);
}
.nodeicon {
  display: grid;
  place-items: center;
  width: 20px;
  height: 20px;
  border-radius: 5px;
  background: var(--muted);
  color: var(--muted-foreground);
  flex: none;
}
.nodeicon.root,
.detail-avatar.root {
  color: var(--warning-foreground);
  background: color-mix(in oklab, var(--warning) 18%, transparent);
}
:global(.dark) .nodeicon.root,
:global(.dark) .detail-avatar.root {
  color: var(--warning);
}
.nodeicon.agent,
.detail-avatar.agent {
  color: var(--studio-violet);
  background: color-mix(in oklab, var(--studio-violet) 16%, transparent);
}
.nodeicon.tool,
.detail-avatar.tool {
  color: var(--studio-cyan);
  background: color-mix(in oklab, var(--studio-cyan) 16%, transparent);
}
.nodeicon.llm,
.detail-avatar.llm {
  color: var(--primary);
  background: color-mix(in oklab, var(--primary) 16%, transparent);
}
.nodeicon.artifact,
.nodeicon.result,
.nodeicon.media,
.detail-avatar.artifact,
.detail-avatar.result,
.detail-avatar.media {
  color: var(--success);
  background: color-mix(in oklab, var(--success) 16%, transparent);
}
.tree-row .title {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
  font-size: 11.5px;
}
.tree-row .rootlabel {
  font-weight: 650;
  color: var(--foreground);
}
.tree-row .sub-label {
  font-weight: 600;
  color: var(--foreground);
}
.tree-row .time {
  font-size: 10px;
  color: var(--muted-foreground);
  flex: none;
  min-width: 22px;
  text-align: right;
  font-variant-numeric: tabular-nums;
}
.tree-row .state-dot {
  display: block;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--muted-foreground);
  flex: none;
  margin-left: 3px;
}
.tree-row .state-dot.done {
  background: var(--success);
}
.tree-row .state-dot.running {
  background: var(--studio-cyan);
  box-shadow: 0 0 0 3px color-mix(in oklab, var(--studio-cyan) 22%, transparent);
  animation: tree-pulse 1.9s infinite;
}
.tree-row .state-dot.failed {
  background: var(--danger);
}
.tree-row .state-dot.pending {
  background: var(--muted-foreground);
  opacity: 0.6;
}
.tree-row .branch-elbow {
  position: absolute;
  left: calc(16px + (var(--depth) - 1) * 18px);
  top: 0;
  height: 16px;
  width: 9px;
  border-left: 1px solid var(--border);
  border-bottom: 1px solid var(--border);
  border-bottom-left-radius: 4px;
  pointer-events: none;
}
.tree-row.depth0 {
  margin-bottom: 4px;
}
.tree-row .sub-count {
  font-size: 9px;
  border: 1px solid var(--border);
  border-radius: 4px;
  color: var(--muted-foreground);
  padding: 0 4px;
  font-variant-numeric: tabular-nums;
}
.empty-search {
  padding: 20px;
  color: var(--muted-foreground);
  text-align: center;
}
.treefooter {
  display: flex;
  align-items: center;
  flex: none;
  gap: 7px;
  height: 35px;
  min-height: 35px;
  padding: 0 11px;
  border-top: 1px solid var(--border);
  font-size: 10px;
  color: var(--muted-foreground);
  background: color-mix(in oklab, var(--surface) 70%, var(--card));
}
.treefooter .live-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--muted-foreground);
  opacity: 0.6;
}
.treefooter .live-dot.active {
  background: var(--success);
  opacity: 1;
  animation: tree-pulse 1.9s infinite;
}
.treefooter .small-btn {
  height: 22px;
  min-width: 22px;
  font-size: 10px;
  margin-left: auto;
}

/* Right Inspector */
.inspector {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  height: 100%;
  overflow: hidden;
  background: var(--card);
}
.inspector-top {
  min-height: 42px;
  height: 42px;
  flex: none;
  padding: 0 16px;
  display: flex;
  align-items: center;
  gap: 7px;
  border-bottom: 1px solid var(--border);
}
.breadcrumb {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  color: var(--muted-foreground);
  font-size: 11px;
  white-space: nowrap;
  overflow: hidden;
}
.breadcrumb .crumb-item {
  background: none;
  border: 0;
  padding: 0;
  color: inherit;
  font: inherit;
  cursor: pointer;
  overflow: hidden;
  text-overflow: ellipsis;
}
.breadcrumb .crumb-item:hover {
  color: var(--foreground);
}
.breadcrumb .crumb-item.current {
  color: var(--foreground);
  font-weight: 600;
}
.breadcrumb .sep {
  color: var(--muted-foreground);
  opacity: 0.6;
}
.inspector-scroll {
  overflow-y: auto;
  overflow-x: hidden;
  flex: 1 1 0%;
  width: 100%;
  min-height: 0;
  scrollbar-width: thin;
  scroll-behavior: smooth;
  overscroll-behavior: contain;
}
.inspector-inner {
  width: 100%;
  padding: 19px 24px 38px;
}
.detail-heading {
  display: flex;
  align-items: flex-start;
  gap: 10px;
}
.detail-avatar {
  width: 34px;
  height: 34px;
  display: grid;
  place-items: center;
  border-radius: 9px;
  flex: none;
}
.detail-heading h2 {
  font-size: 16px;
  line-height: 1.35;
  font-weight: 650;
  margin: 0 0 4px;
  color: var(--foreground);
  word-break: break-word;
}
.metadata {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  color: var(--muted-foreground);
  font-size: 10.5px;
}
.metadata .point {
  opacity: 0.6;
}
.metadata .good {
  color: var(--success);
  font-weight: 600;
}
.metadata .run {
  color: var(--studio-cyan);
  font-weight: 600;
}
.metadata .bad {
  color: var(--danger);
  font-weight: 600;
}
.summary {
  margin: 12px 0 15px;
  font-size: 12px;
  line-height: 1.55;
  color: color-mix(in oklab, var(--foreground) 85%, var(--muted-foreground));
  width: 100%;
  white-space: pre-wrap;
  word-break: break-word;
}
.unknown-alert {
  margin: 0 0 12px;
  padding: 8px 10px;
  border-radius: 6px;
  border: 1px solid color-mix(in oklab, var(--danger) 35%, var(--border));
  background: color-mix(in oklab, var(--danger) 8%, transparent);
  color: var(--danger);
  font-size: 11.5px;
}
.context-line {
  display: flex;
  align-items: center;
  gap: 7px;
  border-top: 1px solid var(--border);
  border-bottom: 1px solid var(--border);
  padding: 9px 0;
  margin-bottom: 12px;
  color: var(--muted-foreground);
  flex-wrap: wrap;
  font-size: 11.5px;
}
.context-line strong {
  color: var(--foreground);
  font-weight: 600;
}
.context-line .tag {
  border: 1px solid var(--border);
  background: color-mix(in oklab, var(--surface) 75%, var(--card));
  padding: 2px 6px;
  border-radius: 4px;
  color: var(--foreground);
  font-size: 10px;
}
.context-line .tag.live {
  color: var(--studio-cyan);
  border-color: color-mix(in oklab, var(--studio-cyan) 40%, var(--border));
  background: color-mix(in oklab, var(--studio-cyan) 10%, transparent);
}
.section-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  color: var(--foreground);
  font-weight: 650;
  margin: 14px 0 8px;
}
.section-title .muted {
  font-weight: 400;
  margin-left: auto;
  font-size: 10px;
  color: var(--muted-foreground);
}
.message-box {
  background: color-mix(in oklab, var(--surface) 75%, var(--card));
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 8px 10px;
  font-size: 11.5px;
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--foreground);
}
.codebox {
  background: color-mix(in oklab, var(--background) 85%, var(--surface));
  border: 1px solid var(--border);
  border-radius: 6px;
  margin-top: 6px;
  overflow: hidden;
}
.codehead {
  height: 26px;
  padding: 0 9px;
  border-bottom: 1px solid var(--border);
  display: flex;
  align-items: center;
  gap: 7px;
  color: var(--muted-foreground);
  font-size: 10px;
  background: color-mix(in oklab, var(--muted) 40%, transparent);
}
.codehead button {
  margin-left: auto;
  background: none;
  border: none;
  display: flex;
  align-items: center;
  color: var(--muted-foreground);
  padding: 2px 4px;
  cursor: pointer;
}
.codehead button:hover {
  color: var(--foreground);
}
.codebox pre {
  margin: 0;
  padding: 10px 11px;
  overflow-x: auto;
  max-height: 220px;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--foreground);
  font-size: 10.5px;
  line-height: 1.55;
  font-family: ui-monospace, SFMono-Regular, Consolas, "Liberation Mono", monospace;
}
.markdown-box {
  background: color-mix(in oklab, var(--surface) 60%, var(--card));
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 10px 12px;
  margin-top: 6px;
}
.result-markdown {
  font-size: 11.5px;
  line-height: 1.6;
  color: var(--foreground);
  overflow-wrap: anywhere;
}
.result-markdown :deep(h1),
.result-markdown :deep(h2),
.result-markdown :deep(h3),
.result-markdown :deep(h4) {
  font-weight: 680;
  line-height: 1.35;
  margin: 0.65em 0 0.35em;
  color: var(--foreground);
}
.result-markdown :deep(h1:first-child),
.result-markdown :deep(h2:first-child),
.result-markdown :deep(h3:first-child),
.result-markdown :deep(p:first-child) {
  margin-top: 0;
}
.result-markdown :deep(code) {
  background: var(--muted);
  padding: 1px 4px;
  border-radius: 4px;
  font-size: 0.92em;
}
.result-markdown :deep(pre) {
  background: var(--background);
  padding: 8px 10px;
  border-radius: 6px;
  overflow-x: auto;
  font-size: 10.5px;
}

/* Media & Artifacts */
.media-list {
  display: grid;
  gap: 10px;
  margin-top: 6px;
}
.media-item-card {
  border-radius: 8px;
  border: 1px solid var(--border);
  background: color-mix(in oklab, var(--surface) 65%, var(--card));
  padding: 8px 10px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.media-item-preview {
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 6px;
  background: #0b111c;
  overflow: hidden;
  max-height: 260px;
}
.media-card-image,
.media-card-video {
  display: block;
  max-width: 100%;
  max-height: 260px;
  width: auto;
  height: auto;
  object-fit: contain;
}
.media-item-meta {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  font-size: 11px;
}
.artifact-row {
  display: flex;
  align-items: center;
  gap: 8px;
  border: 1px solid var(--border);
  padding: 7px 10px;
  border-radius: 6px;
  background: color-mix(in oklab, var(--surface) 65%, var(--card));
  color: var(--foreground);
  font-size: 11px;
  margin: 6px 0;
}
.artifact-row strong {
  font-weight: 550;
}
.artifact-row small {
  margin-left: auto;
  color: var(--muted-foreground);
}
.child-btn {
  width: 100%;
  cursor: pointer;
  text-align: left;
  transition: background 0.12s, border-color 0.12s;
}
.child-btn:hover {
  background: var(--muted);
  border-color: color-mix(in oklab, var(--primary) 40%, var(--border));
}

/* Events Feed */
.feed {
  border-left: 1px solid var(--border);
  margin-left: 8px;
}
.event {
  position: relative;
  display: grid;
  grid-template-columns: 64px minmax(0, 1fr);
  gap: 8px;
  padding: 0 0 13px 16px;
}
.event:last-child {
  padding-bottom: 0;
}
.event::before {
  content: "";
  position: absolute;
  top: 4px;
  left: -4px;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--muted-foreground);
  border: 1px solid var(--card);
}
.event.output::before {
  background: var(--success);
}
.event.error::before {
  background: var(--danger);
}
.event.processing::before {
  background: var(--studio-cyan);
  box-shadow: 0 0 0 3px color-mix(in oklab, var(--studio-cyan) 20%, transparent);
}
.timestamp {
  color: var(--muted-foreground);
  font-size: 10px;
  line-height: 17px;
  font-variant-numeric: tabular-nums;
}
.eventbody {
  min-width: 0;
}
.eventname {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 11.5px;
  color: var(--foreground);
  font-weight: 570;
  margin: 0 0 4px;
}
.eventname svg {
  color: var(--muted-foreground);
}
.message {
  color: color-mix(in oklab, var(--foreground) 80%, var(--muted-foreground));
  font-size: 11.5px;
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;
}
.error-code-box {
  margin-top: 10px;
  padding: 8px 10px;
  border-radius: 6px;
  background: color-mix(in oklab, var(--danger) 8%, transparent);
  border: 1px solid color-mix(in oklab, var(--danger) 30%, var(--border));
  color: var(--danger);
  font-family: ui-monospace, SFMono-Regular, Consolas, monospace;
  font-size: 11px;
  word-break: break-all;
}

.mobile-only {
  display: none;
}

@keyframes tree-pulse {
  50% {
    opacity: 0.45;
  }
}

@media (max-width: 800px) {
  .workspace {
    grid-template-columns: 260px minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr);
    height: calc(100dvh - 180px);
    min-height: 420px;
  }
  .stats {
    gap: 13px;
    padding: 0 12px;
  }
  .stat {
    margin: 0;
    padding: 0 13px 0 0;
  }
  .inspector-inner {
    padding: 16px;
  }
  .search {
    margin: 6px 7px;
  }
  .tree-row {
    height: 30px;
  }
}

@media (max-width: 570px) {
  .workspace {
    grid-template-columns: 1fr;
    grid-template-rows: minmax(0, 1fr);
    position: relative;
  }
  .treepane {
    width: 100%;
    border-right: 0;
  }
  .inspector {
    display: none;
  }
  .tree-shell.detail-open .treepane {
    display: none;
  }
  .tree-shell.detail-open .inspector {
    display: flex;
  }
  .mobile-only {
    display: inline-flex;
  }
}
</style>

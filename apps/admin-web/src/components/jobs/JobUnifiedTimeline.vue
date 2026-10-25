<script setup lang="ts">
import { ChevronRight, Download, Eye } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { agentJobsApi } from '@/api/agent-jobs'
import type { JobArtifact } from '@/api/types/agent-jobs'
import { jobResultReport, type JobReport } from '@/domain/jobs/result-report'
import {
  groupTimelineActivities,
  layoutDagCanvas,
  type ActivityFilter,
  type ActivityKind,
  type UnifiedActivity,
} from '@/domain/jobs/unified-activity'
import { useI18n } from '@/composables/useI18n'
import ArtifactPreviewDialog, { artifactPreview } from './artifacts/ArtifactPreviewDialog.vue'
import JobActivityInspector from './JobActivityInspector.vue'

const props = defineProps<{
  jobId?: string
  activities: UnifiedActivity[]
  selectedId: string | null
  filter: ActivityFilter
  roles: string[]
}>()
const emit = defineEmits<{
  select: [id: string]
  filter: [value: ActivityFilter]
}>()
const { t } = useI18n()

const zoom = ref(0.72)
const viewportRef = ref<HTMLElement | null>(null)
const dragging = ref(false)
let dragStart = { x: 0, y: 0, scrollLeft: 0, scrollTop: 0 }

const inspectorOpen = ref(false)
const inspectedId = ref<string | null>(props.selectedId)

watch(
  () => props.selectedId,
  nextId => {
    inspectedId.value = nextId
  },
)

const activeSelectedId = computed(() => inspectedId.value ?? props.selectedId)
const inspectedActivity = computed(() => {
  if (!inspectorOpen.value || !activeSelectedId.value) return null
  return props.activities.find(activity => activity.id === activeSelectedId.value) ?? null
})

function onNodeClick(id: string) {
  inspectedId.value = id
  inspectorOpen.value = true
  emit('select', id)
}

function closeInspector() {
  inspectorOpen.value = false
}

const userOpenOverrides = ref<Record<string, boolean>>({})
const prevFinished = new Map<string, boolean>()

function agentFinished(activities: UnifiedActivity[]): boolean {
  return activities.some(
    activity =>
      activity.kind === 'agent' ||
      ['succeeded', 'failed', 'cancelled', 'interrupted'].includes(activity.agentRun?.status ?? ''),
  )
}

watch(
  () => props.activities,
  activities => {
    const groups = groupTimelineActivities(activities)
    let changed = false
    const nextOverrides = { ...userOpenOverrides.value }
    for (const group of groups) {
      if (!group.child) continue
      const finished = agentFinished(group.activities)
      const prev = prevFinished.get(group.id)
      if (prev !== undefined && prev !== finished && group.id in nextOverrides) {
        delete nextOverrides[group.id]
        changed = true
      }
      prevFinished.set(group.id, finished)
    }
    if (changed) {
      userOpenOverrides.value = nextOverrides
    }
  },
  { immediate: true },
)

const collapsedLaneIds = computed(() => {
  const collapsed = new Set<string>()
  for (const group of groupTimelineActivities(props.activities)) {
    if (!group.child) continue
    const isOpen = userOpenOverrides.value[group.id] ?? !agentFinished(group.activities)
    if (!isOpen) collapsed.add(group.id)
  }
  return collapsed
})

const layout = computed(() => layoutDagCanvas(props.activities, collapsedLaneIds.value))

function onSummaryClick(event: MouseEvent, groupId: string) {
  event.preventDefault()
  const currentlyOpen = !collapsedLaneIds.value.has(groupId)
  const nextOpen = !currentlyOpen
  userOpenOverrides.value = {
    ...userOpenOverrides.value,
    [groupId]: nextOpen,
  }
  const details = (event.currentTarget as HTMLElement | null)?.closest('details')
  if (details) {
    details.open = nextOpen
  }
}

const MIN_ZOOM = 0.35
const MAX_ZOOM = 1.6

function setZoom(next: number) {
  zoom.value = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Number(next.toFixed(4))))
}
function stepZoom(delta: number) {
  setZoom(zoom.value + delta)
}
function fitCanvas() {
  const cw = viewportRef.value?.clientWidth || 1100
  const ch = viewportRef.value?.clientHeight || 618
  const fitW = (cw - 24) / layout.value.width
  const fitH = (ch - 24) / layout.value.height
  setZoom(Math.max(0.43, Math.min(0.95, Math.min(fitW, fitH))))
  if (viewportRef.value) {
    viewportRef.value.scrollLeft = 0
    viewportRef.value.scrollTop = 0
  }
}
function onWheel(event: WheelEvent) {
  if (event.deltaY === 0) return
  const viewport = viewportRef.value
  const prevZoom = zoom.value
  const deltaPixels =
    event.deltaMode === 1 ? event.deltaY * 33 : event.deltaMode === 2 ? event.deltaY * 100 : event.deltaY
  const sensitivity = event.ctrlKey ? 0.004 : 0.001
  const delta = Math.min(0.2, Math.max(-0.2, -deltaPixels * sensitivity))
  setZoom(prevZoom + delta)
  const nextZoom = zoom.value
  if (!viewport || nextZoom === prevZoom) return
  const rect = viewport.getBoundingClientRect()
  if (rect.width > 0 && rect.height > 0) {
    const offsetX = event.clientX - rect.left
    const offsetY = event.clientY - rect.top
    const ratio = nextZoom / prevZoom
    viewport.scrollLeft = Math.max(0, Math.round((viewport.scrollLeft + offsetX) * ratio - offsetX))
    viewport.scrollTop = Math.max(0, Math.round((viewport.scrollTop + offsetY) * ratio - offsetY))
  }
}

function onPointerDown(event: PointerEvent) {
  const target = event.target as HTMLElement | null
  if (!viewportRef.value || target?.closest('button, summary, details, .result-card-body')) return
  dragging.value = true
  dragStart = {
    x: event.clientX,
    y: event.clientY,
    scrollLeft: viewportRef.value.scrollLeft,
    scrollTop: viewportRef.value.scrollTop,
  }
}
function onPointerMove(event: PointerEvent) {
  if (!dragging.value || !viewportRef.value) return
  viewportRef.value.scrollLeft = dragStart.scrollLeft - (event.clientX - dragStart.x)
  viewportRef.value.scrollTop = dragStart.scrollTop - (event.clientY - dragStart.y)
}
function onPointerUp() {
  dragging.value = false
}

const previewedArtifact = ref<JobArtifact | null>(null)
const downloadingArtifactId = ref<string | null>(null)

function resultReport(activity: UnifiedActivity): JobReport {
  return jobResultReport(activity.jobResult)
}

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

function isRootRole(role: string | null): boolean {
  return role === 'orchestrator' || role === 'root' || Boolean(role?.endsWith('_root'))
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
function clock(input: string): string {
  const value = new Date(input)
  return Number.isFinite(value.getTime()) ? value.toLocaleTimeString([], { hour12: false }) : '—'
}
function nodeIcon(kind: ActivityKind): string {
  switch (kind) {
    case 'llm':
      return '✳'
    case 'plugin':
      return '⬡'
    case 'fork':
      return '⑂'
    case 'join':
      return '⑃'
    case 'result':
      return '📄'
    default:
      return '✓'
  }
}
function footerLeft(activity: UnifiedActivity): string {
  if (activity.llm) {
    const known = activity.llm.inputTokens !== null || activity.llm.outputTokens !== null
    if (known) {
      const total = (activity.llm.inputTokens ?? 0) + (activity.llm.outputTokens ?? 0)
      return total >= 1000 ? `${(total / 1000).toFixed(1)}k tokens` : `${total} tokens`
    }
    return activity.llm.model ?? activity.llm.provider
  }
  if (activity.kind === 'plugin') return activity.step?.step_key ?? 'plugin'
  if (activity.kind === 'fork' || activity.kind === 'join') return activity.forkGroupId ?? activity.kind
  if ((activity.kind === 'job' || activity.kind === 'result') && activity.artifacts?.length) {
    return `${activity.artifacts.length} ${t('jobUi.artifacts').toLowerCase()}`
  }
  return activity.step?.step_key ?? activity.agentRunId ?? activity.kind
}
function footerRight(activity: UnifiedActivity): string {
  if (activity.llm) {
    const ms = activity.llm.totalDurationMs ?? activity.llm.requestDurationMs
    return ms !== null ? `${(ms / 1000).toFixed(1)}s` : activity.status
  }
  if (activity.kind === 'fork' || activity.kind === 'join') {
    return `${activity.memberCount ?? 0} children`
  }
  return clock(activity.time)
}
</script>

<template>
  <section class="studio-panel dag-surface min-w-0 overflow-hidden" :aria-label="t('jobUi.unifiedTimeline')">
    <div class="surface-head flex flex-wrap items-center justify-between gap-2 border-b px-4 py-2.5">
      <div class="mini-tools flex flex-wrap items-center gap-2">
        <strong class="text-xs font-semibold">◎ {{ t('jobUi.executionMap') }}</strong>
        <span class="small-badge">{{ t('jobUi.dagFlowBadge') }}</span>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <div class="filterbar flex flex-wrap gap-1" role="group" :aria-label="t('jobUi.timelineFilter')">
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
        <div class="mini-tools flex items-center gap-1.5">
          <span class="subtle tiny tabular-nums" data-zoom-readout>{{ Math.round(zoom * 100) }}%</span>
          <button
            type="button"
            class="toolbutton"
            data-zoom-out
            :title="t('jobUi.zoomOut')"
            :aria-label="t('jobUi.zoomOut')"
            @click="stepZoom(-0.1)"
          >
            −
          </button>
          <button
            type="button"
            class="toolbutton"
            data-zoom-in
            :title="t('jobUi.zoomIn')"
            :aria-label="t('jobUi.zoomIn')"
            @click="stepZoom(0.1)"
          >
            +
          </button>
          <button
            type="button"
            class="toolbutton"
            data-zoom-fit
            :title="t('jobUi.zoomFit')"
            :aria-label="t('jobUi.zoomFit')"
            @click="fitCanvas"
          >
            ⤢
          </button>
        </div>
      </div>
    </div>

    <div
      v-if="filter.kind === 'agent'"
      class="flex items-center justify-between gap-3 border-b px-4 py-2 text-xs text-muted-foreground"
    >
      <span>{{ t('jobUi.agentRunFilter') }}</span>
      <button type="button" class="text-studio-violet hover:underline" @click="emit('filter', { kind: 'all' })">
        {{ t('jobUi.clearTimelineFilter') }}
      </button>
    </div>

    <p v-if="!activities.length" class="p-6 text-sm text-muted-foreground">{{ t('jobUi.noTimelineActivity') }}</p>

    <div v-else class="canvas-body">
      <div
        ref="viewportRef"
        class="canvas-viewport"
        @wheel.prevent="onWheel"
        @pointerdown="onPointerDown"
        @pointermove="onPointerMove"
        @pointerup="onPointerUp"
        @pointercancel="onPointerUp"
        @pointerleave="onPointerUp"
      >
        <div
          class="canvas-scale-box"
          :style="{
            width: Math.round(layout.width * zoom) + 'px',
            height: Math.round(layout.height * zoom) + 'px',
          }"
        >
          <div
            class="canvas-stage"
            :style="{
              width: layout.width + 'px',
              height: layout.height + 'px',
              transform: 'scale(' + zoom + ')',
            }"
          >
            <svg
              class="edge-layer"
              :viewBox="'0 0 ' + layout.width + ' ' + layout.height"
              aria-hidden="true"
            >
              <defs>
                <marker id="arrow" markerWidth="5" markerHeight="5" refX="4.5" refY="2.5" orient="auto">
                  <path d="M0 0 L5 2.5 L0 5 Z" fill="#687897" />
                </marker>
              </defs>
              <path
                v-for="edge in layout.edges"
                :key="edge.id"
                :class="edge.kind === 'default' ? undefined : edge.kind"
                :d="edge.d"
                marker-end="url(#arrow)"
              />
            </svg>

            <ol class="execution-timeline">
              <li
                v-for="item in layout.groups"
                :key="item.group.id"
                class="timeline-group"
                :class="{ 'child-group': item.group.child }"
                :data-agent-run-id="item.group.child ? item.group.id : undefined"
              >
                <div
                  v-if="item.group.child && item.lane"
                  class="lane-bg"
                  :class="[item.lane.theme, { collapsed: item.lane.collapsed }]"
                  :style="{
                    left: item.lane.x + 'px',
                    top: item.lane.y + 'px',
                    width: item.lane.width + 'px',
                    height: item.lane.height + 'px',
                  }"
                />

                <component
                  :is="item.group.child ? 'details' : 'div'"
                  class="lane-details"
                  :open="item.group.child ? !collapsedLaneIds.has(item.group.id) : undefined"
                >
                  <summary
                    v-if="item.group.child && item.lane"
                    class="lane-title cursor-pointer list-none"
                    :class="item.lane.theme"
                    :style="{
                      left: item.lane.x + 18 + 'px',
                      top: item.lane.y + 12 + 'px',
                      width: item.lane.width - 36 + 'px',
                    }"
                    @click="onSummaryClick($event, item.group.id)"
                  >
                    <span class="flex items-center gap-1">
                      <ChevronRight class="branch-toggle size-3.5 shrink-0 transition-transform" aria-hidden="true" />
                      <span class="truncate">◉ {{ t('jobUi.subAgent') }} · {{ roleName(item.lane.role) }}</span>
                      <span class="ml-1 truncate font-mono text-[10px] opacity-80">{{ item.group.id }}</span>
                      <span class="lane-badge ml-auto shrink-0">{{ item.nodes.length }}</span>
                    </span>
                    <span
                      v-if="item.group.activities[0]?.agentRun?.goal_excerpt"
                      class="lane-goal mt-1 block truncate font-normal text-[#9eaac1]"
                    >
                      {{ item.group.activities[0].agentRun.goal_excerpt }}
                    </span>
                  </summary>

                  <ol v-show="!item.lane?.collapsed">
                    <li v-for="node in item.nodes" :key="node.id" class="timeline-row">
                      <div
                        v-if="node.activity.kind === 'result'"
                        class="graph-node result-card"
                        :class="{
                          selected: activeSelectedId === node.activity.id,
                        }"
                        :data-step="node.activity.id"
                        :data-kind="node.activity.kind"
                        :style="{
                          left: node.x + 'px',
                          top: node.y + 'px',
                          width: node.width + 'px',
                          height: node.height + 'px',
                        }"
                        @click="onNodeClick(node.activity.id)"
                      >
                        <div class="node-header">
                          <div class="flex min-w-0 items-center gap-2">
                            <span class="node-icon" aria-hidden="true">{{ nodeIcon(node.activity.kind) }}</span>
                            <b class="truncate text-xs" :title="t('jobUi.result')">{{ t('jobUi.result') }}</b>
                          </div>
                          <span class="node-status" :class="node.activity.status">
                            ● {{ node.activity.status }}
                          </span>
                        </div>
                        <div class="result-card-body" @wheel.stop>
                          <article
                            v-if="resultReport(node.activity).kind === 'markdown'"
                            class="result-markdown"
                            data-result-markdown
                            v-html="(resultReport(node.activity) as { kind: 'markdown'; html: string }).html"
                          />
                          <pre
                            v-else-if="resultReport(node.activity).kind === 'json'"
                            class="result-json"
                          >{{ (resultReport(node.activity) as { kind: 'json'; text: string }).text }}</pre>
                          <p v-else class="desc">{{ node.activity.note }}</p>
                        </div>
                        <div v-if="node.activity.artifacts?.length" class="result-artifacts" @wheel.stop>
                          <div
                            v-for="art in node.activity.artifacts"
                            :key="art.id"
                            class="result-artifact-row"
                          >
                            <span class="truncate" :title="art.filename || art.name || art.id">
                              📎 {{ art.filename || art.name || art.id }}
                            </span>
                            <div v-if="jobId" class="flex shrink-0 items-center gap-1">
                              <button
                                type="button"
                                class="artifact-btn"
                                :disabled="downloadingArtifactId !== null"
                                @click.stop="downloadArtifact(art)"
                              >
                                <Download class="size-3" />{{ t('jobUi.download') }}
                              </button>
                              <button
                                v-if="canPreviewArtifact(art)"
                                type="button"
                                class="artifact-btn"
                                data-canvas-artifact-preview
                                @click.stop="previewedArtifact = art"
                              >
                                <Eye class="size-3" />Preview
                              </button>
                            </div>
                          </div>
                        </div>
                        <div class="node-footer">
                          <span class="truncate">{{ footerLeft(node.activity) }}</span>
                          <span class="shrink-0">{{ footerRight(node.activity) }}</span>
                        </div>
                      </div>
                      <button
                        v-else
                        type="button"
                        class="graph-node"
                        :class="{
                          selected: activeSelectedId === node.activity.id,
                          running: node.activity.status === 'running',
                          pending: node.activity.status === 'cancelled' || node.activity.status === 'skipped',
                        }"
                        :data-step="node.activity.id"
                        :data-kind="node.activity.kind"
                        :style="{
                          left: node.x + 'px',
                          top: node.y + 'px',
                          width: node.width + 'px',
                          height: node.height + 'px',
                        }"
                        :aria-current="activeSelectedId === node.activity.id ? 'true' : undefined"
                        @click="onNodeClick(node.activity.id)"
                      >
                        <div class="node-header">
                          <span class="node-icon" aria-hidden="true">{{ nodeIcon(node.activity.kind) }}</span>
                          <span class="node-status" :class="node.activity.status">
                            {{ node.activity.status === 'completed' || node.activity.status === 'running' ? '● ' : '○ ' }}{{ node.activity.status }}
                          </span>
                        </div>
                        <b class="truncate" :title="node.activity.title">{{ node.activity.title }}</b>
                        <span class="desc line-clamp-2">
                          {{
                            isRootRole(node.activity.role) && !item.group.child
                              ? t('jobUi.rootAgent') + ' · ' + node.activity.note
                              : node.activity.note
                          }}
                        </span>
                        <span
                          v-for="child in node.activity.childRuns"
                          :key="child.agent_run_id"
                          class="desc mt-1 truncate text-[#83e0e9]"
                        >
                          {{ roleName(child.role) }} · {{ child.goal_excerpt }}
                          <span class="font-mono text-[9px]">{{ child.agent_run_id }}</span>
                        </span>
                        <span
                          v-if="node.activity.artifacts?.length"
                          class="desc mt-1 truncate text-[#83e0e9]"
                          :title="node.activity.artifacts.map(a => a.filename || a.name || a.id).join(', ')"
                        >
                          📎 {{ node.activity.artifacts[0]?.filename || node.activity.artifacts[0]?.name || node.activity.artifacts[0]?.id }}<template v-if="node.activity.artifacts.length > 1"> +{{ node.activity.artifacts.length - 1 }}</template>
                        </span>
                        <span
                          v-if="node.activity.llm && node.activity.step"
                          class="desc mt-1"
                          :class="
                            node.activity.step.attempt_state === 'unknown_outcome' ||
                            node.activity.step.status === 'failed'
                              ? 'text-[#fb8490]'
                              : ''
                          "
                        >
                          {{ t('jobUi.llmCheckpointProgress') }} · {{ node.activity.step.step_key }} · {{ node.activity.step.status }}<template v-if="node.activity.step.attempt_state === 'unknown_outcome'"> · unknown_outcome</template>
                        </span>
                        <div class="node-footer">
                          <span class="truncate">{{ footerLeft(node.activity) }}</span>
                          <span class="shrink-0">{{ footerRight(node.activity) }}</span>
                        </div>
                      </button>
                    </li>
                  </ol>
                </component>
              </li>
            </ol>
          </div>
        </div>
      </div>
      <div class="canvas-help">{{ t('jobUi.canvasHelp') }}</div>

      <aside
        v-if="inspectedActivity"
        class="canvas-inspector"
        data-canvas-inspector
      >
        <JobActivityInspector
          :job-id="jobId"
          :activity="inspectedActivity"
          open
          closable
          @close="closeInspector"
        />
      </aside>
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
.small-badge {
  border-radius: 5px;
  background: #202a3a;
  color: #97a6be;
  padding: 4px 6px;
  font-size: 10px;
}
.tiny {
  font-size: 10px;
}
.subtle {
  color: var(--muted-foreground);
}
.toolbutton {
  width: 29px;
  height: 29px;
  border-radius: 8px;
  background: #1d2738;
  border: 1px solid #33425a;
  font-size: 13px;
  color: #a5b3ce;
  display: inline-grid;
  place-items: center;
}
.toolbutton:hover {
  background: #29364b;
}
.filter {
  padding: 5px 9px;
  border-radius: 7px;
  background: #151c2a;
  border: 1px solid #2a364c;
  color: #899ab6;
  font-size: 10px;
  transition: border-color 0.15s, background 0.15s, color 0.15s;
}
.filter:hover {
  border-color: #4f5f82;
}
.filter.active {
  border-color: #6055aa;
  background: #2a2448;
  color: #c5b8ff;
}
.canvas-body {
  display: flex;
  align-items: stretch;
  position: relative;
  height: 618px;
  background-color: #101725;
}
.canvas-viewport {
  flex: 1 1 0%;
  min-width: 0;
  height: 100%;
  overflow: auto;
  position: relative;
  display: flex;
  background-color: #101725;
  background-image: radial-gradient(#4c597647 1px, transparent 1px);
  background-size: 20px 20px;
  scrollbar-color: #3e4860 transparent;
  cursor: grab;
}
.canvas-viewport:active {
  cursor: grabbing;
}
.canvas-scale-box {
  margin: auto;
  flex-shrink: 0;
}
.canvas-inspector {
  width: 340px;
  max-width: 45%;
  height: 100%;
  overflow-y: auto;
  border-left: 1px solid #283348;
  background: #131b2c;
  scrollbar-color: #3e4860 transparent;
  flex-shrink: 0;
  z-index: 4;
}
.canvas-inspector :deep(.inspector) {
  border: 0;
  border-radius: 0;
  background: transparent;
  box-shadow: none;
}
.canvas-inspector :deep(.surface-head) {
  position: sticky;
  top: 0;
  background: #151e30;
  border-color: #283348;
  z-index: 1;
}
.canvas-stage {
  position: relative;
  transform-origin: 0 0;
}
.execution-timeline,
.execution-timeline ol {
  list-style: none;
  margin: 0;
  padding: 0;
}
.lane-bg {
  position: absolute;
  border-radius: 15px;
  border: 1px dashed #2a66726b;
  background: linear-gradient(90deg, #16414c24, #1d2e3f11);
  pointer-events: none;
  transition: width 0.15s, height 0.15s, top 0.15s, left 0.15s;
}
.lane-bg.collapsed {
  border-style: solid;
  border-color: #2f728088;
  background: linear-gradient(145deg, #17364266, #14223388);
  box-shadow: 0 8px 24px #0004;
}
.lane-bg.document {
  border-color: #6d6c3c59;
  background: linear-gradient(90deg, #51451e1b, #2c30300d);
}
.lane-bg.document.collapsed {
  border-color: #82864588;
  background: linear-gradient(145deg, #3c3a1f66, #1c222b88);
}
.lane-details {
  position: static;
}
details.lane-details:not([open]) > ol {
  display: none;
}
details.lane-details[open] > .lane-title .branch-toggle {
  transform: rotate(90deg);
}
.lane-title {
  position: absolute;
  z-index: 2;
  font-size: 11px;
  letter-spacing: 0.5px;
  font-weight: 700;
  color: #87ccd8;
  user-select: none;
}
.lane-title::-webkit-details-marker {
  display: none;
}
.lane-title.document {
  color: #bacb85;
}
.lane-badge {
  border-radius: 999px;
  background: #202d42;
  border: 1px solid #354764;
  color: #b8c7df;
  padding: 1px 7px;
  font-size: 9px;
  font-weight: 650;
}
.lane-goal {
  font-size: 9.5px;
  letter-spacing: normal;
}
.edge-layer {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  overflow: visible;
  pointer-events: none;
}
.edge-layer path {
  fill: none;
  stroke: #566482;
  stroke-width: 2.5;
  stroke-linecap: round;
}
.edge-layer path.branch {
  stroke: #5d9cac;
}
.edge-layer path.pending {
  stroke: #4f5b71;
  stroke-dasharray: 6 7;
}
.edge-layer path.animated {
  stroke: #59c8d8;
  stroke-dasharray: 5 8;
  animation: dash 2.1s linear infinite;
}
@keyframes dash {
  to {
    stroke-dashoffset: -27;
  }
}
.graph-node {
  position: absolute;
  width: 164px;
  height: 122px;
  padding: 11px 12px;
  border-radius: 12px;
  border: 1px solid #3a4560;
  background: linear-gradient(145deg, #212b3e, #161e2d);
  color: #e9edf7;
  text-align: left;
  box-shadow: 0 9px 27px #0004;
  transition: border 0.15s, box-shadow 0.15s, transform 0.15s;
  z-index: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.graph-node:hover {
  transform: translateY(-2px);
  border-color: #8998c5;
}
.graph-node.selected {
  border-color: #baa2ff;
  box-shadow: 0 0 0 2px #a28af525, 0 12px 30px #0005;
}
.graph-node.running {
  border-color: #3a8798;
  background: linear-gradient(145deg, #183745, #19243a);
}
.graph-node.pending {
  border-style: dashed;
  opacity: 0.77;
}
.node-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 5px;
  margin-bottom: 9px;
}
.node-icon {
  display: grid;
  place-items: center;
  width: 29px;
  height: 29px;
  border-radius: 8px;
  background: #4a3f75;
  color: #d0c2ff;
  font-size: 13px;
}
.graph-node[data-kind='plugin'] .node-icon {
  background: #244958;
  color: #8de4ef;
}
.graph-node[data-kind='fork'] .node-icon,
.graph-node[data-kind='join'] .node-icon {
  background: #58472a;
  color: #fad092;
}
.graph-node[data-kind='job'] .node-icon,
.graph-node[data-kind='result'] .node-icon,
.graph-node[data-kind='agent'] .node-icon,
.graph-node[data-kind='plan'] .node-icon {
  background: #2b5549;
  color: #a5ecd6;
}
.result-card {
  padding: 13px 14px;
  border-color: #3c695d;
  background: linear-gradient(155deg, #1b283b, #131c2b);
  cursor: default;
}
.result-card-body {
  flex: 1 1 0%;
  min-height: 0;
  overflow-y: auto;
  padding: 8px 10px;
  border-radius: 8px;
  background: #0f1623b8;
  border: 1px solid #2b3850;
  scrollbar-color: #3e4860 transparent;
  user-select: text;
}
.result-markdown {
  font-size: 11px;
  line-height: 1.55;
  color: #e3eaf6;
  overflow-wrap: anywhere;
}
.result-markdown :deep(h1),
.result-markdown :deep(h2),
.result-markdown :deep(h3),
.result-markdown :deep(h4) {
  font-weight: 680;
  line-height: 1.35;
  margin: 0.7em 0 0.35em;
  color: #ffffff;
}
.result-markdown :deep(h1:first-child),
.result-markdown :deep(h2:first-child),
.result-markdown :deep(h3:first-child),
.result-markdown :deep(p:first-child) {
  margin-top: 0;
}
.result-markdown :deep(h1) {
  font-size: 1.25em;
}
.result-markdown :deep(h2) {
  font-size: 1.14em;
}
.result-markdown :deep(h3) {
  font-size: 1.05em;
}
.result-markdown :deep(p),
.result-markdown :deep(ul),
.result-markdown :deep(ol) {
  margin: 0.4em 0;
}
.result-markdown :deep(ul),
.result-markdown :deep(ol) {
  padding-left: 1.25em;
}
.result-markdown :deep(li) {
  margin: 0.2em 0;
}
.result-markdown :deep(code) {
  background: #1e293d;
  padding: 1px 4px;
  border-radius: 4px;
  font-size: 0.92em;
}
.result-markdown :deep(pre) {
  background: #141d2d;
  padding: 6px 8px;
  border-radius: 6px;
  overflow-x: auto;
  font-size: 10px;
}
.result-json {
  margin: 0;
  font-size: 10px;
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-word;
  color: #d7e2f4;
}
.result-artifacts {
  margin-top: 7px;
  max-height: 68px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.result-artifact-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
  padding: 4px 7px;
  border-radius: 6px;
  background: #182336;
  border: 1px solid #2b3b55;
  font-size: 9.5px;
  color: #83e0e9;
}
.artifact-btn {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 2px 6px;
  border-radius: 5px;
  background: #223149;
  border: 1px solid #394e70;
  color: #d6e2f5;
  font-size: 9px;
  cursor: pointer;
}
.artifact-btn:hover {
  background: #2c3f5e;
}
.graph-node b {
  display: block;
  font-size: 11px;
  margin-bottom: 3px;
  line-height: 1.25;
}
.graph-node .desc {
  font-size: 9.5px;
  line-height: 1.45;
  color: #9eaac1;
  display: block;
}
.node-status {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 9px;
  color: #8a98b2;
  text-transform: uppercase;
  letter-spacing: 0.2px;
}
.node-status.completed {
  color: #5fd9a6;
}
.node-status.running {
  color: #58c9db;
}
.node-status.pending,
.node-status.cancelled,
.node-status.skipped {
  color: #8f9bb3;
}
.node-status.failed,
.node-status.unknown,
.node-status.interrupted {
  color: #fb8490;
}
.node-footer {
  margin-top: auto;
  padding-top: 6px;
  border-top: 1px solid #3d486033;
  font-size: 9px;
  color: #8b9db9;
  display: flex;
  justify-content: space-between;
  gap: 6px;
}
.canvas-help {
  position: absolute;
  bottom: 15px;
  left: 16px;
  background: #101826e8;
  color: #9aa9c3;
  border: 1px solid #354159;
  padding: 6px 10px;
  border-radius: 6px;
  font-size: 10px;
  pointer-events: none;
  z-index: 3;
}
@media (max-width: 850px) {
  .canvas-body {
    height: 470px;
  }
  .canvas-inspector {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(320px, 88%);
    max-width: 88%;
    box-shadow: -12px 0 28px #0007;
  }
}
</style>

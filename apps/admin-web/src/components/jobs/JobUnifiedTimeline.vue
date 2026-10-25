<script setup lang="ts">
import { computed, ref } from 'vue'
import {
  layoutDagCanvas,
  type ActivityFilter,
  type ActivityKind,
  type UnifiedActivity,
} from '@/domain/jobs/unified-activity'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{
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

const layout = computed(() => layoutDagCanvas(props.activities))

function setZoom(next: number) {
  zoom.value = Math.min(1.2, Math.max(0.43, Number(next.toFixed(2))))
}
function stepZoom(delta: number) {
  setZoom(zoom.value + delta)
}
function fitCanvas() {
  const cw = viewportRef.value?.clientWidth || 1100
  setZoom(Math.max(0.43, Math.min(0.95, (cw - 15) / layout.value.width)))
  if (viewportRef.value) {
    viewportRef.value.scrollLeft = 0
    viewportRef.value.scrollTop = 0
  }
}

function onPointerDown(event: PointerEvent) {
  const target = event.target as HTMLElement | null
  if (!viewportRef.value || target?.closest('button, summary, details')) return
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

function agentFinished(activities: UnifiedActivity[]): boolean {
  return activities.some(
    activity =>
      activity.kind === 'agent' ||
      ['succeeded', 'failed', 'cancelled', 'interrupted'].includes(activity.agentRun?.status ?? ''),
  )
}
function isRootRole(role: string | null): boolean {
  return role === 'orchestrator' || role === 'root'
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
  <section class="studio-panel dag-surface min-w-0 overflow-hidden" :aria-label="t('jobUi.dagCanvasBadge')">
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

    <div
      v-else
      ref="viewportRef"
      class="canvas-viewport"
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
              <marker id="arrow" markerWidth="7" markerHeight="7" refX="6" refY="3" orient="auto">
                <path d="M0 0 L6 3 L0 6 Z" fill="#687897" />
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
              <template v-if="item.group.child && item.lane">
                <div
                  class="lane-bg"
                  :class="item.lane.theme"
                  :style="{
                    left: item.lane.x + 'px',
                    top: item.lane.y + 'px',
                    width: item.lane.width + 'px',
                    height: item.lane.height + 'px',
                  }"
                />
                <details
                  class="lane-details"
                  :open="!agentFinished(item.group.activities)"
                  :style="{
                    left: item.lane.x + 23 + 'px',
                    top: item.lane.y + 11 + 'px',
                    maxWidth: item.lane.width - 36 + 'px',
                  }"
                >
                  <summary class="lane-title cursor-pointer list-none" :class="item.lane.theme">
                    <span>◉ {{ t('jobUi.subAgent') }} · {{ roleName(item.lane.role) }}</span>
                    <span class="ml-2 font-mono text-[10px] opacity-80">{{ item.group.id }}</span>
                    <span
                      v-if="item.group.activities[0]?.agentRun?.goal_excerpt"
                      class="lane-goal mt-0.5 block truncate font-normal text-[#9eaac1]"
                    >
                      {{ item.group.activities[0].agentRun.goal_excerpt }}
                    </span>
                  </summary>
                </details>
              </template>

              <ol>
                <li v-for="node in item.nodes" :key="node.id" class="timeline-row">
                  <button
                    type="button"
                    class="graph-node"
                    :class="{
                      selected: selectedId === node.activity.id,
                      running: node.activity.status === 'running',
                      pending: node.activity.status === 'cancelled' || node.activity.status === 'skipped',
                    }"
                    :data-step="node.activity.id"
                    :data-kind="node.activity.kind"
                    :style="{ left: node.x + 'px', top: node.y + 'px' }"
                    :aria-current="selectedId === node.activity.id ? 'true' : undefined"
                    @click="emit('select', node.activity.id)"
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
            </li>
          </ol>
        </div>
      </div>
      <div class="canvas-help">{{ t('jobUi.canvasHelp') }}</div>
    </div>
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
.canvas-viewport {
  height: 618px;
  overflow: auto;
  position: relative;
  background-color: #101725;
  background-image: radial-gradient(#4c597647 1px, transparent 1px);
  background-size: 20px 20px;
  scrollbar-color: #3e4860 transparent;
  cursor: grab;
}
.canvas-viewport:active {
  cursor: grabbing;
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
}
.lane-bg.document {
  border-color: #6d6c3c59;
  background: linear-gradient(90deg, #51451e1b, #2c30300d);
}
.lane-details {
  position: absolute;
  z-index: 2;
}
.lane-title {
  font-size: 11px;
  letter-spacing: 0.7px;
  font-weight: 700;
  color: #87ccd8;
}
.lane-title::-webkit-details-marker {
  display: none;
}
.lane-title.document {
  color: #bacb85;
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
  min-height: 108px;
  padding: 12px;
  border-radius: 12px;
  border: 1px solid #3a4560;
  background: linear-gradient(145deg, #212b3e, #161e2d);
  color: #e9edf7;
  text-align: left;
  box-shadow: 0 9px 27px #0004;
  transition: border 0.15s, box-shadow 0.15s, transform 0.15s;
  z-index: 1;
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
.graph-node[data-kind='agent'] .node-icon,
.graph-node[data-kind='plan'] .node-icon {
  background: #2b5549;
  color: #a5ecd6;
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
  margin-top: 9px;
  padding-top: 7px;
  border-top: 1px solid #3d486033;
  font-size: 9px;
  color: #8b9db9;
  display: flex;
  justify-content: space-between;
  gap: 6px;
}
.canvas-help {
  position: sticky;
  bottom: 15px;
  left: 16px;
  display: inline-block;
  margin: 0 0 15px 16px;
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
  .canvas-viewport {
    height: 470px;
  }
}
</style>

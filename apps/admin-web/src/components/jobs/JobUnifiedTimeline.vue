<script setup lang="ts">
import { computed } from 'vue'
import { groupTimelineActivities } from '@/domain/jobs/unified-activity'
import { BrainCircuit, CheckCircle2, ChevronRight, GitFork, GitMerge, Puzzle, Workflow } from '@lucide/vue'
import type { ActivityFilter, UnifiedActivity } from '@/domain/jobs/unified-activity'
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
const groups = computed(() => groupTimelineActivities(props.activities))
function agentFinished(activities: UnifiedActivity[]): boolean {
  return activities.some(activity => activity.kind === 'agent' ||
    ['succeeded', 'failed', 'cancelled', 'interrupted'].includes(activity.agentRun?.status ?? ''))
}
function roleName(role: string | null): string {
  switch (role) {
    case 'orchestrator': return t('jobUi.agentOrchestrator')
    case 'research': return t('jobUi.agentResearch')
    case 'document': return t('jobUi.agentDocument')
    case 'general': return t('jobUi.agentGeneral')
    default: return t('jobUi.agentLegacy')
  }
}
function clock(input: string): string {
  const value = new Date(input)
  return Number.isFinite(value.getTime()) ? value.toLocaleTimeString([], { hour12: false }) : '—'
}
function tone(status: UnifiedActivity['status']): string {
  if (status === 'failed' || status === 'unknown') return 'text-danger'
  if (status === 'completed') return 'text-success'
  return 'text-studio-cyan'
}
</script>

<template>
  <section class="studio-panel min-w-0">
    <div class="flex flex-wrap items-center justify-between gap-2 border-b p-4">
      <h3 class="flex items-center gap-2 text-sm font-semibold"><Workflow class="size-4 text-studio-violet" />{{ t('jobUi.unifiedTimeline') }}</h3>
      <div class="flex flex-wrap gap-1" role="group" :aria-label="t('jobUi.timelineFilter')">
        <button type="button" class="rounded-md border px-2 py-1 text-xs transition-colors hover:border-primary/40" :class="filter.kind === 'all' ? 'border-primary/50 bg-primary/10 text-studio-violet' : 'text-muted-foreground'" :aria-pressed="filter.kind === 'all'" @click="emit('filter', { kind: 'all' })">{{ t('jobUi.timelineAll') }}</button>
        <button v-for="role in roles" :key="role" type="button" class="rounded-md border px-2 py-1 text-xs transition-colors hover:border-primary/40" :class="filter.kind === 'role' && filter.role === role ? 'border-primary/50 bg-primary/10 text-studio-violet' : 'text-muted-foreground'" :aria-pressed="filter.kind === 'role' && filter.role === role" @click="emit('filter', { kind: 'role', role })">{{ roleName(role) }}</button>
      </div>
    </div>
    <div v-if="filter.kind === 'agent'" class="flex items-center justify-between gap-3 border-b px-4 py-2 text-xs text-muted-foreground">
      <span>{{ t('jobUi.agentRunFilter') }}</span>
      <button type="button" class="text-studio-violet hover:underline" @click="emit('filter', { kind: 'all' })">{{ t('jobUi.clearTimelineFilter') }}</button>
    </div>
    <p v-if="!activities.length" class="p-6 text-sm text-muted-foreground">{{ t('jobUi.noTimelineActivity') }}</p>
    <ol v-else class="execution-timeline p-4">
      <li v-for="group in groups" :key="group.id" class="timeline-group" :class="{ 'child-group': group.child }" :data-agent-run-id="group.child ? group.id : undefined">
        <svg v-if="group.child" class="branch-connector branch-entry" viewBox="0 0 36 24" aria-hidden="true">
          <path d="M0 0 Q0 12 12 12 H24 Q36 12 36 24" />
        </svg>
        <svg v-if="group.child" class="branch-connector branch-return" viewBox="0 0 36 24" aria-hidden="true">
          <path d="M36 0 Q36 12 24 12 H12 Q0 12 0 24" />
        </svg>
        <component :is="group.child ? 'details' : 'div'" :open="group.child ? !agentFinished(group.activities) : undefined">
          <summary v-if="group.child" class="branch-heading list-none cursor-pointer rounded-md py-2 text-xs text-teal-600 dark:text-teal-400">
            <ChevronRight class="branch-toggle mr-1 inline size-3.5" aria-hidden="true" />
            <span class="font-semibold uppercase">{{ t('jobUi.subAgent') }} · {{ roleName(group.activities[0]?.role ?? null) }}</span>
            <span class="ml-2 break-all font-mono text-[10px]">{{ group.id }}</span>
            <span v-if="group.activities[0]?.agentRun?.goal_excerpt" class="mt-1 block break-words text-muted-foreground">{{ group.activities[0].agentRun.goal_excerpt }}</span>
          </summary>
          <ol>
            <li v-for="activity in group.activities" :key="activity.id" class="timeline-row relative grid grid-cols-[65px_28px_minmax(0,1fr)] gap-2 pb-3 last:pb-0 sm:grid-cols-[82px_28px_minmax(0,1fr)]">
              <time class="pt-4 text-[10px] tabular-nums text-muted-foreground" :datetime="activity.time">{{ clock(activity.time) }}</time>
              <div class="relative flex justify-center pt-3">
                <div class="row-rail rail-before" aria-hidden="true" /><div class="row-rail rail-after" aria-hidden="true" />
                <div class="relative z-10 grid size-7 place-items-center rounded-lg" :class="[tone(activity.status), group.child ? 'bg-teal-500/10' : 'bg-primary/10']">
                  <BrainCircuit v-if="activity.kind === 'llm'" class="size-4" />
                  <Puzzle v-else-if="activity.kind === 'plugin'" class="size-4" />
                  <GitFork v-else-if="activity.kind === 'fork'" class="size-4" />
                  <GitMerge v-else-if="activity.kind === 'join'" class="size-4" />
                  <CheckCircle2 v-else class="size-4" />
                </div>
              </div>
              <button type="button" class="mb-2 min-w-0 rounded-lg border bg-card p-3 text-left transition-colors hover:border-primary/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" :class="selectedId === activity.id ? 'border-primary/60 ring-1 ring-primary/20' : ''" :aria-current="selectedId === activity.id ? 'true' : undefined" @click="emit('select', activity.id)">
                <div class="flex flex-wrap items-center justify-between gap-2">
                  <span class="flex min-w-0 flex-wrap items-center gap-2">
                    <span class="rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase" :class="group.child ? 'bg-teal-500/10 text-teal-600 dark:text-teal-400' : 'bg-primary/10 text-studio-violet'">{{ group.child ? t('jobUi.subAgent') : activity.role === 'orchestrator' ? t('jobUi.rootAgent') : '' }}{{ group.child || activity.role === 'orchestrator' ? ' · ' : '' }}{{ roleName(activity.role) }}</span>
                    <strong class="break-words text-xs">{{ activity.title }}</strong>
                  </span>
                  <span class="text-[10px] font-semibold capitalize" :class="tone(activity.status)">{{ activity.status }}</span>
                </div>
                <p class="mt-1 break-words text-xs text-muted-foreground">{{ activity.note }}</p>
                <p v-for="child in activity.childRuns" :key="child.agent_run_id" class="mt-2 break-words text-xs text-teal-600 dark:text-teal-400">{{ roleName(child.role) }} · {{ child.goal_excerpt }} <span class="font-mono text-[10px]">{{ child.agent_run_id }}</span></p>
                <p v-if="activity.llm" class="mt-2 break-all text-[11px] text-muted-foreground">{{ activity.llm.provider }} · {{ activity.llm.model ?? '—' }} · {{ activity.llm.transport ?? '—' }}</p>
                <p v-if="activity.llm && activity.step" class="mt-2 text-[11px]" :class="activity.step.attempt_state === 'unknown_outcome' || activity.step.status === 'failed' ? 'text-danger' : 'text-muted-foreground'">
                  {{ t('jobUi.llmCheckpointProgress') }} · {{ activity.step.step_key }} · {{ activity.step.status }}<span v-if="activity.step.attempt_state === 'unknown_outcome'"> · unknown_outcome</span>
                </p>
              </button>
            </li>
          </ol>
        </component>
      </li>
    </ol>
  </section>
</template>

<style scoped>
.execution-timeline { --axis: 87px; --rail: var(--color-studio-violet, #8b5cf6); }
.timeline-group { position: relative; }
.row-rail { position: absolute; width: 1px; background: var(--rail); opacity: .45; }
.rail-before { top: 0; height: 8px; }
.rail-after { top: 44px; bottom: 0; }
.timeline-row:not(:last-child) .rail-after { bottom: -12px; }
.timeline-group:first-child:not(.child-group) .rail-before,
.timeline-group:last-child:not(.child-group) .rail-after { display: none; }
.child-group { padding: 24px 0 24px 36px; --rail: #14b8a6; }
.child-group::before {
  content: ''; position: absolute; left: calc(var(--axis) - .5px); top: 0; bottom: 0;
  border-left: 1px solid var(--color-studio-violet, #8b5cf6); opacity: .45;
}
.branch-connector {
  position: absolute; left: var(--axis); width: 36px; height: 24px;
  fill: none; stroke: var(--rail); stroke-width: 1; opacity: .45;
  overflow: visible; pointer-events: none;
}
.branch-entry { top: 0; }
.branch-return { bottom: 0; }
.branch-heading { position: relative; margin-left: calc(var(--axis) + 8px); }
.branch-heading::-webkit-details-marker { display: none; }
[open] > .branch-heading .branch-toggle { transform: rotate(90deg); }
.branch-heading::after {
  content: ''; position: absolute; left: -8.5px; top: 0; bottom: 0;
  border-left: 1px solid var(--rail); opacity: .45;
}
@media (min-width: 640px) { .execution-timeline { --axis: 104px; } }
</style>

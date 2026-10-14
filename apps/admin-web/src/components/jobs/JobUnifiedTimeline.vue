<script setup lang="ts">
import { BrainCircuit, CheckCircle2, GitFork, GitMerge, Puzzle, Workflow } from '@lucide/vue'
import type { ActivityFilter, UnifiedActivity } from '@/domain/jobs/unified-activity'
import { useI18n } from '@/composables/useI18n'

defineProps<{
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
    <ol v-else class="p-4">
      <li v-for="(activity, index) in activities" :key="activity.id" class="relative grid grid-cols-[65px_28px_minmax(0,1fr)] gap-2 pb-3 last:pb-0 sm:grid-cols-[82px_28px_minmax(0,1fr)]">
        <time class="pt-4 text-[10px] tabular-nums text-muted-foreground" :datetime="activity.time">{{ clock(activity.time) }}</time>
        <div class="relative flex justify-center pt-3">
          <div v-if="index < activities.length - 1" class="absolute bottom-[-12px] top-8 w-px bg-border" aria-hidden="true" />
          <div class="relative z-10 grid size-7 place-items-center rounded-lg bg-primary/10" :class="tone(activity.status)">
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
              <span class="rounded bg-primary/10 px-1.5 py-0.5 text-[10px] font-semibold uppercase text-studio-violet">{{ roleName(activity.role) }}</span>
              <strong class="break-words text-xs">{{ activity.title }}</strong>
            </span>
            <span class="text-[10px] font-semibold capitalize" :class="tone(activity.status)">{{ activity.status }}</span>
          </div>
          <p class="mt-1 break-words text-xs text-muted-foreground">{{ activity.note }}</p>
          <p v-if="activity.llm" class="mt-2 break-all text-[11px] text-muted-foreground">{{ activity.llm.provider }} · {{ activity.llm.model ?? '—' }} · {{ activity.llm.transport ?? '—' }}</p>
        </button>
      </li>
    </ol>
  </section>
</template>

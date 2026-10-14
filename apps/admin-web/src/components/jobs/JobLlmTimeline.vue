<script setup lang="ts">
import { Activity, AlertCircle, BrainCircuit, CheckCircle2, ChevronDown, Clock3, Cpu, PlayCircle, RefreshCw, Sparkles, Workflow } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { AdminJob, JobEvent } from '@/api/types/agent-jobs'
import { llmTokenSummary, projectLlmRuns, type LlmRun } from '@/domain/jobs/llm-activity'
import { Badge } from '@/components/ui/badge'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{ job: AdminJob; events: JobEvent[] }>()
const { t } = useI18n()
const runs = computed(() => projectLlmRuns(props.events, props.job.status))
const selected = ref<string | null>(null)
const expanded = ref(new Set<string>())
const now = ref(Date.now())
let ticker: ReturnType<typeof setInterval> | undefined
watch(() => props.job.id, () => { selected.value = null; expanded.value = new Set() })
watch(runs, values => {
  if (selected.value && values.some(run => run.id === selected.value)) return
  selected.value = values[0]?.id ?? null
  if (selected.value) expanded.value.add(selected.value)
}, { immediate: true })
onMounted(() => { ticker = setInterval(() => { if (runs.value.some(r => r.status === 'running')) now.value = Date.now() }, 1000) })
onBeforeUnmount(() => { if (ticker) clearInterval(ticker) })
const active = computed(() => runs.value.filter(run => run.status === 'running').length)
const failed = computed(() => runs.value.filter(run => run.status === 'failed').length)
const tokens = computed(() => llmTokenSummary(runs.value))
const repairCount = computed(() => runs.value.filter(run => run.phase === 'repair').length)
const selectedRun = computed(() => runs.value.find(run => run.id === selected.value) ?? null)
const isJobTerminal = computed(() => ['succeeded', 'partial_success', 'failed', 'cancelled'].includes(props.job.status))
const finishedEvent = computed(() => [...props.events].reverse().find(e => e.event_type === 'job_finished'))
const elapsed = computed(() => {
  const start = Date.parse(props.job.created_at)
  const lastObserved = props.events.length ? props.events[props.events.length - 1]?.created_at : null
  const end = isJobTerminal.value
    ? Date.parse(finishedEvent.value?.created_at ?? lastObserved ?? props.job.created_at)
    : now.value
  if (!Number.isFinite(start) || !Number.isFinite(end)) return '—'
  const seconds = Math.max(0, Math.floor((end - start) / 1000))
  return String(Math.floor(seconds / 60)).padStart(2, '0') + ':' + String(seconds % 60).padStart(2, '0')
})
const hasPlan = computed(() => props.events.some(e => e.event_type === 'plan_accepted') || props.job.plan_revision > 0)
const workerSteps = computed(() => props.job.steps.filter(s => s.plugin_id === null))
const workerComplete = computed(() => workerSteps.value.length > 0 && workerSteps.value.every(s => ['succeeded', 'failed', 'skipped', 'cancelled'].includes(s.status)))
const stageCount = computed(() => Number(hasPlan.value) + Number(workerComplete.value) + Number(isJobTerminal.value))
const progress = computed(() => Math.round(stageCount.value * 100 / 3))

function choose(run: LlmRun) {
  if (selected.value === run.id) {
    if (expanded.value.has(run.id)) expanded.value.delete(run.id)
    else expanded.value.add(run.id)
  } else {
    selected.value = run.id
    expanded.value.add(run.id)
  }
}
function role(run: LlmRun) {
  if (run.role === 'worker') return t('jobUi.llmWorker')
  if (run.phase === 'repair') return t('jobUi.llmRepair')
  if (run.phase === 'replan') return t('jobUi.llmReplan')
  return t('jobUi.llmPlanner')
}
function stateLabel(status: LlmRun['status']) {
  if (status === 'completed') return t('jobUi.llmCompleted')
  if (status === 'failed') return t('jobUi.llmFailed')
  if (status === 'cancelled') return t('jobUi.llmCancelled')
  if (status === 'interrupted') return t('jobUi.llmInterrupted')
  return t('jobUi.llmRunning')
}
function statusTone(status: LlmRun['status']): 'success' | 'danger' | 'secondary' | 'outline' {
  return status === 'completed' ? 'success' : status === 'failed' ? 'danger' : status === 'running' ? 'secondary' : 'outline'
}
function eventName(event: JobEvent) {
  switch (event.event_type) {
    case 'llm_run_started': return t('jobUi.llmStarted')
    case 'llm_provider_acquired': return t('jobUi.llmProviderAcquired')
    case 'llm_request_started': return t('jobUi.llmDispatched')
    case 'llm_response_received': return t('jobUi.llmReceived')
    case 'llm_run_finished': return event.payload.validation_path ? t('jobUi.llmValidationFailed') : t('jobUi.llmFinished')
    default: return event.event_type
  }
}
function eventNote(event: JobEvent) {
  if (event.event_type === 'llm_run_finished') return String(event.payload.validation_code ?? event.payload.error_code ?? '')
  if (event.event_type === 'llm_provider_acquired') return (event.payload.provider_wait_ms ?? '—') + ' ms'
  if (event.event_type === 'llm_request_started') return (event.payload.capacity_wait_ms ?? '—') + ' ms'
  if (event.event_type === 'llm_response_received') return (event.payload.request_duration_ms ?? '—') + ' ms'
  return ''
}
function time(input: string) {
  const value = new Date(input)
  return Number.isFinite(value.getTime()) ? value.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' }) : '—'
}
function duration(run: LlmRun) {
  if (run.totalDurationMs !== null) return (run.totalDurationMs / 1000).toFixed(2) + ' s'
  if (run.requestDurationMs !== null) return (run.requestDurationMs / 1000).toFixed(2) + ' s'
  if (run.status === 'running') {
    const started = Date.parse(run.startedAt)
    if (Number.isFinite(started)) return Math.max(0,Math.floor((now.value - started)/1000)) + ' s'
  }
  return '—'
}
function usage(run: LlmRun) {
  if (run.inputTokens === null && run.outputTokens === null) return '—'
  return String(run.inputTokens ?? '—') + ' / ' + String(run.outputTokens ?? '—')
}
function progressLabel(done: boolean, activeStage: boolean) {
  return done ? t('jobUi.llmCompleted') : activeStage ? t('jobUi.llmRunning') : t('jobUi.llmPending')
}
</script>

<template>
  <section class="space-y-4">
    <div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
      <div class="studio-panel p-4">
        <div class="flex items-center justify-between text-xs text-muted-foreground"><span>{{ t('jobUi.llmRuns') }}</span><BrainCircuit class="size-4 text-studio-violet" /></div>
        <p class="mt-2 text-2xl font-bold tracking-tight">{{ runs.length }}</p>
        <p class="mt-1 text-xs text-muted-foreground">{{ active }} {{ t('jobUi.llmRunning') }} · {{ failed }} {{ t('jobUi.llmFailed') }}</p>
      </div>
      <div class="studio-panel p-4">
        <div class="flex items-center justify-between text-xs text-muted-foreground"><span>{{ t('jobUi.llmRepair') }}</span><RefreshCw class="size-4 text-studio-violet" /></div>
        <p class="mt-2 text-2xl font-bold tracking-tight">{{ repairCount }}</p>
        <p class="mt-1 text-xs text-muted-foreground">{{ hasPlan ? t('jobUi.llmPlanAccepted') : t('jobUi.llmPlanPending') }}</p>
      </div>
      <div class="studio-panel p-4">
        <div class="flex items-center justify-between text-xs text-muted-foreground"><span>{{ t('jobUi.llmTokens') }}</span><Cpu class="size-4 text-studio-violet" /></div>
        <p class="mt-2 text-2xl font-bold tracking-tight">{{ tokens.known ? tokens.tokens.toLocaleString() : '—' }}</p>
        <p class="mt-1 text-xs text-muted-foreground">{{ tokens.known }} / {{ tokens.total }} {{ t('jobUi.llmUsageAvailable') }}</p>
      </div>
      <div class="studio-panel p-4">
        <div class="flex items-center justify-between text-xs text-muted-foreground"><span>{{ t('jobUi.llmElapsed') }}</span><Clock3 class="size-4 text-studio-violet" /></div>
        <p class="mt-2 text-2xl font-bold tabular-nums tracking-tight">{{ elapsed }}</p>
        <p class="mt-1 text-xs text-muted-foreground">{{ isJobTerminal ? t('jobUi.llmFinished') : t('jobUi.llmRunning') }}</p>
      </div>
    </div>

    <div class="grid items-start gap-4 xl:grid-cols-[minmax(0,1.62fr)_minmax(255px,.76fr)]">
      <section class="min-w-0">
        <div class="mb-3 flex flex-wrap items-center justify-between gap-2">
          <div>
            <h2 class="flex items-center gap-2 font-semibold"><Activity class="size-4 text-studio-violet" />{{ t('jobUi.llmTimeline') }}</h2>
            <p class="mt-1 text-xs text-muted-foreground">{{ t('jobUi.llmTimelineSub') }}</p>
          </div>
          <span class="text-xs text-muted-foreground">{{ runs.length }} {{ t('jobUi.llmRuns') }}</span>
        </div>
        <div v-if="!runs.length" class="studio-panel p-6 text-sm text-muted-foreground">{{ t('jobUi.llmEmpty') }}</div>
        <ol v-else class="space-y-0">
          <li v-for="(run,index) in runs" :key="run.id" class="relative grid grid-cols-[52px_minmax(0,1fr)]">
            <div v-if="index + 1 < runs.length" class="absolute bottom-0 left-[26px] top-9 w-px bg-border" aria-hidden="true" />
            <div class="relative z-10 flex flex-col items-center pt-3">
              <div class="grid size-7 place-items-center rounded-lg" :class="run.status === 'failed' ? 'bg-danger/10 text-danger' : run.status === 'running' ? 'bg-studio-cyan/10 text-studio-cyan' : 'bg-primary/10 text-studio-violet'">
                <AlertCircle v-if="run.status === 'failed'" class="size-4" />
                <PlayCircle v-else-if="run.status === 'running'" class="size-4" />
                <CheckCircle2 v-else class="size-4" />
              </div>
              <span class="mt-2 text-[10px] text-muted-foreground">{{ time(run.startedAt).slice(0,5) }}</span>
            </div>
            <button type="button" class="studio-panel mb-3 min-w-0 p-4 text-left transition-colors hover:border-primary/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" :class="selected === run.id ? 'border-primary/50 ring-1 ring-primary/15' : ''" :aria-expanded="expanded.has(run.id)" :aria-current="selected === run.id ? 'true' : undefined" @click="choose(run)">
              <div class="flex flex-wrap items-center justify-between gap-2">
                <strong class="text-sm">{{ t('jobUi.llmRun') }} {{ String(run.number).padStart(2,'0') }} · {{ role(run) }}</strong>
                <Badge :variant="statusTone(run.status)">{{ stateLabel(run.status) }}</Badge>
              </div>
              <p class="mt-2 text-xs text-muted-foreground">{{ run.stepKey || role(run) }}</p>
              <div class="mt-3 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                <span class="rounded-md bg-primary/10 px-2 py-1 font-medium text-primary">{{ run.provider }}</span>
                <span>{{ run.model ?? '—' }}</span>
                <span class="rounded border px-1.5 py-0.5">{{ run.transport ?? '—' }}</span>
              </div>
              <div v-if="expanded.has(run.id)" class="mt-3 space-y-2 border-t pt-3">
                <div v-for="stage in run.events" :key="stage.sequence" class="flex items-start gap-2 text-xs">
                  <CheckCircle2 v-if="stage.event_type !== 'llm_run_finished' || run.status === 'completed'" class="mt-0.5 size-3.5 shrink-0 text-success" />
                  <AlertCircle v-else class="mt-0.5 size-3.5 shrink-0 text-danger" />
                  <span class="min-w-0 flex-1">{{ eventName(stage) }}<span v-if="stage.payload.validation_path" class="ml-2 break-all font-mono text-danger">{{ stage.payload.validation_path }}</span></span>
                  <span class="shrink-0 text-muted-foreground">{{ eventNote(stage) }}</span>
                </div>
                <p v-if="run.errorCode" class="break-all font-mono text-xs text-danger">{{ run.errorCode }}</p>
              </div>
              <div class="mt-3 flex justify-between gap-2 border-t pt-2 text-[11px] text-muted-foreground">
                <span>{{ t('jobUi.llmAttempt') }} {{ run.attempt }}<span v-if="run.stepKey"> · {{ run.stepKey }}</span></span>
                <span class="flex items-center gap-1"><ChevronDown class="size-3 transition-transform" :class="expanded.has(run.id) ? 'rotate-180' : ''" />{{ expanded.has(run.id) ? t('jobUi.llmCollapse') : t('jobUi.llmViewStages') }}</span>
              </div>
            </button>
          </li>
        </ol>
      </section>

      <aside class="min-w-0 space-y-4">
        <section class="studio-panel p-5">
          <div class="flex items-center justify-between gap-2">
            <h2 class="font-semibold">{{ t('jobUi.llmSelected') }}</h2>
            <Badge variant="outline">{{ t('jobUi.llmReadOnly') }}</Badge>
          </div>
          <template v-if="selectedRun">
            <div class="mt-4 grid grid-cols-2 gap-4 text-xs">
              <div><p class="text-muted-foreground">{{ t('jobUi.llmProvider') }}</p><p class="mt-1 break-all font-semibold">{{ selectedRun.provider }}</p></div>
              <div><p class="text-muted-foreground">{{ t('jobUi.llmModel') }}</p><p class="mt-1 break-all font-semibold">{{ selectedRun.model ?? '—' }}</p></div>
              <div><p class="text-muted-foreground">{{ t('jobUi.llmTransport') }}</p><p class="mt-1 font-semibold">{{ selectedRun.transport ?? '—' }}</p></div>
              <div><p class="text-muted-foreground">{{ t('jobUi.llmSource') }}</p><p class="mt-1 font-semibold">{{ selectedRun.providerSource }}</p></div>
              <div><p class="text-muted-foreground">{{ t('jobUi.llmRole') }}</p><p class="mt-1 font-semibold">{{ role(selectedRun) }}</p></div>
              <div><p class="text-muted-foreground">{{ t('jobUi.llmRunId') }}</p><p class="mt-1 break-all font-mono text-[11px]">{{ selectedRun.id }}</p></div>
            </div>
            <div class="mt-5 border-t pt-4">
              <h3 class="text-xs font-bold uppercase tracking-wide text-muted-foreground">{{ t('jobUi.llmOutputMetrics') }}</h3>
              <div class="mt-3 flex justify-between gap-2 text-xs"><span class="text-muted-foreground">{{ t('jobUi.llmDuration') }}</span><strong>{{ duration(selectedRun) }}</strong></div>
              <div class="mt-3 flex justify-between gap-2 text-xs"><span class="text-muted-foreground">{{ t('jobUi.llmTokens') }}</span><strong>{{ usage(selectedRun) }}</strong></div>
              <div v-if="selectedRun.errorCode" class="mt-3 break-all rounded-lg bg-danger/5 p-3 font-mono text-xs text-danger">{{ selectedRun.errorCode }} <span v-if="selectedRun.validationPath">{{ selectedRun.validationPath }}</span></div>
            </div>
          </template>
          <p v-else class="mt-4 text-xs text-muted-foreground">{{ t('jobUi.llmSelectHint') }}</p>
        </section>
        <section class="studio-panel p-5">
          <div class="mb-4 flex items-center justify-between gap-2">
            <h2 class="flex items-center gap-2 font-semibold"><Workflow class="size-4 text-studio-violet" />{{ t('jobUi.llmProgress') }}</h2>
            <span class="text-xs text-muted-foreground">{{ stageCount }} / 3</span>
          </div>
          <div class="h-1.5 overflow-hidden rounded-full bg-muted"><div class="h-full rounded-full bg-primary transition-all" :style="{ width: progress + '%' }" /></div>
          <div class="mt-4 space-y-3 text-xs">
            <div class="flex justify-between gap-2"><span class="flex items-center gap-2"><Sparkles class="size-3.5" />{{ t('jobUi.llmPlanningProgress') }}</span><span class="text-muted-foreground">{{ progressLabel(hasPlan, !hasPlan && !isJobTerminal) }}</span></div>
            <div class="flex justify-between gap-2"><span class="flex items-center gap-2"><BrainCircuit class="size-3.5" />{{ t('jobUi.llmWorkerProgress') }}</span><span class="text-muted-foreground">{{ progressLabel(workerComplete, hasPlan && !workerComplete && !isJobTerminal) }}</span></div>
            <div class="flex justify-between gap-2"><span class="flex items-center gap-2"><CheckCircle2 class="size-3.5" />{{ t('jobUi.llmCheckpointProgress') }}</span><span class="text-muted-foreground">{{ progressLabel(isJobTerminal, false) }}</span></div>
          </div>
        </section>
      </aside>
    </div>
  </section>
</template>

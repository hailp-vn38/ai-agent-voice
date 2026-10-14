<script setup lang="ts">
import { Activity, AlertTriangle, BrainCircuit, Clock3, Cpu, Download, FileText, Layers, Puzzle, RefreshCw, RotateCcw, Square, Workflow } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { agentJobsApi } from '@/api/agent-jobs'
import { listenJobChanges } from '@/api/job-updates'
import { useAuthStore } from '@/stores/auth'
import { formatApiError, isApiError } from '@/api/errors'
import type { AdminJob, JobEvent, JobStatus, StepStatus } from '@/api/types/agent-jobs'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import JobUnifiedTimeline from '@/components/jobs/JobUnifiedTimeline.vue'
import JobActivityInspector from '@/components/jobs/JobActivityInspector.vue'
import { llmTokenSummary, projectLlmRuns } from '@/domain/jobs/llm-activity'
import { projectUnifiedActivity, visibleActivities, type ActivityFilter } from '@/domain/jobs/unified-activity'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()
const auth = useAuthStore()
const jobId = computed(() => String(route.params.jobId ?? ''))
const job = ref<AdminJob | null>(null)
const events = ref<JobEvent[]>([])
const loading = ref(true)
const refreshing = ref(false)
const busy = ref(false)
const error = ref('')
const eventsError = ref('')
const notice = ref('')
const retryOpen = ref(false)
let controller: AbortController | undefined
let streamController: AbortController | undefined
let refreshTimer: ReturnType<typeof setTimeout> | undefined
let fallbackTimer: ReturnType<typeof setInterval> | undefined
let reconciling = false
let dirty = false
let mounted = false
let eventCursor = 0
let retryRequest = { id: '', key: '' }

const isTerminal = (status: JobStatus) => ['succeeded', 'partial_success', 'failed', 'cancelled'].includes(status)
const canRetry = computed(() => job.value && ['failed', 'cancelled'].includes(job.value.status))
const planningEvents = computed(() => events.value.filter(x => x.event_type.startsWith('plan_')))
const executionEvents = computed(() => events.value.filter(x => !x.event_type.startsWith('plan_') && !x.event_type.startsWith('llm_')))
const unknownOutcome = computed(() => job.value?.steps.some(step => step.attempt_state === 'unknown_outcome') ?? false)

const activityFilter = ref<ActivityFilter>({ kind: 'all' })
const selectedActivityId = ref<string | null>(null)
const llmRuns = computed(() => job.value ? projectLlmRuns(events.value, job.value.status) : [])
const tokenUsage = computed(() => llmTokenSummary(llmRuns.value))
const activeLlm = computed(() => llmRuns.value.filter(run => run.status === 'running').length)
const pluginCalls = computed(() => job.value?.steps.filter(step => step.plugin_id && (step.attempt_state || step.status !== 'pending')).length ?? 0)
const agentRuns = computed(() => [...(job.value?.agent_runs ?? [])].sort((a, b) => a.run_index - b.run_index))
const roleFilters = computed(() => [...new Set(agentRuns.value.map(run => run.role))])
const allActivity = computed(() => job.value ? projectUnifiedActivity(job.value, events.value) : [])
const filteredActivity = computed(() => visibleActivities(allActivity.value, activityFilter.value))
const selectedActivity = computed(() => filteredActivity.value.find(item => item.id === selectedActivityId.value) ?? null)
const coverage = computed(() => job.value?.coverage ?? null)
const sourcePercentage = computed(() => coverage.value && coverage.value.required_sources_count > 0
  ? Math.min(100, Math.round(coverage.value.verified_sources_count * 100 / coverage.value.required_sources_count)) : 0)
watch(filteredActivity, rows => {
  if (!rows.some(item => item.id === selectedActivityId.value)) selectedActivityId.value = rows[0]?.id ?? null
}, { immediate: true })
function filterAgent(id: string) { activityFilter.value = { kind: 'agent', id } }
function roleName(role: string) {
  if (role === 'orchestrator') return t('jobUi.agentOrchestrator')
  if (role === 'research') return t('jobUi.agentResearch')
  if (role === 'document') return t('jobUi.agentDocument')
  return t('jobUi.agentGeneral')
}

const tone = (status: JobStatus): 'success' | 'danger' | 'secondary' | 'outline' =>
  status === 'succeeded' ? 'success' : status === 'failed' ? 'danger' :
    status === 'partial_success' || status === 'cancelled' ? 'outline' : 'secondary'
const stepTone = (status: StepStatus): 'success' | 'danger' | 'secondary' =>
  status === 'succeeded' ? 'success' : status === 'failed' ? 'danger' : 'secondary'
const statusLabel = (status: JobStatus) => t(('jobUi.jobStatus.' + status) as 'jobUi.jobStatus.queued')
function date(value?: string) {
  if (!value) return '—'
  const timestamp = new Date(value)
  return Number.isFinite(timestamp.getTime()) ? timestamp.toLocaleString() : value
}
function summary(result: AdminJob['result']) {
  if (!result) return ''
  return typeof result.summary === 'string' ? result.summary : JSON.stringify(result, null, 2)
}
function budget() {
  if (!job.value) return '—'
  return Object.keys(job.value.budget_used ?? {}).length
    ? JSON.stringify(job.value.budget_used, null, 2) : '—'
}
async function load(resetEvents = false) {
  controller?.abort()
  const pending = new AbortController()
  controller = pending
  if (!job.value || resetEvents) loading.value = true
  else refreshing.value = true
  error.value = ''
  if (resetEvents) {
    job.value = null
    events.value = []
    eventCursor = 0
  }
  const id = jobId.value
  try {
    const loaded = await agentJobsApi.job(id, pending.signal)
    if (pending.signal.aborted || id !== jobId.value) return
    job.value = loaded
    eventsError.value = ''
    // Fetch the actual durable event log, including planning retries.
    while (true) {
      const response = await agentJobsApi.events(id, eventCursor, pending.signal)
      if (pending.signal.aborted || id !== jobId.value) return
      const received = response.items.filter(entry => entry.sequence > eventCursor)
      events.value.push(...received)
      if (response.items.length) eventCursor = response.items[response.items.length - 1]!.sequence
      if (response.items.length < 100) break
    }
  } catch (cause) {
    if (!pending.signal.aborted) {
      if (isApiError(cause) && cause.status === 404) {
        job.value = null
        events.value = []
        error.value = formatApiError(cause)
      } else if (!job.value) error.value = formatApiError(cause)
      else eventsError.value = formatApiError(cause)
    }
  } finally {
    if (controller === pending) { loading.value = false; refreshing.value = false }
  }
}
async function downloadArtifact(artifactId: string) {
  if (!job.value) return
  error.value = ''
  try {
    const blob = await agentJobsApi.artifact(job.value.id, artifactId)
    const url = URL.createObjectURL(blob)
    const link = document.createElement('a')
    link.href = url
    link.download = artifactId
    link.click()
    window.setTimeout(() => URL.revokeObjectURL(url), 1000)
  } catch (cause) {
    error.value = formatApiError(cause)
  }
}
async function cancel() {
  if (!job.value || busy.value) return
  busy.value = true
  error.value = ''
  try {
    job.value = await agentJobsApi.cancelJob(job.value.id)
    notice.value = t('jobUi.cancelAccepted')
    await load()
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}
async function retry() {
  if (!job.value || busy.value) return
  const id = job.value.id
  busy.value = true
  error.value = ''
  try {
    if (retryRequest.id !== id) retryRequest = { id, key: crypto.randomUUID() }
    const created = await agentJobsApi.retryJob(id, retryRequest.key)
    retryRequest = { id: '', key: '' }
    await router.push({ name: 'job-detail', params: { jobId: created.id } })
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}
function reconcile() {
  dirty = true
  if (refreshTimer || reconciling) return
  refreshTimer = setTimeout(() => {
    refreshTimer = undefined
    if (reconciling) return
    void (async () => {
      reconciling = true
      try {
        while (dirty && mounted) {
          dirty = false
          await load()
        }
      } finally {
        reconciling = false
        if (dirty && mounted) reconcile()
      }
    })()
  }, 200)
}
function startStream() {
  streamController?.abort()
  if (!mounted) return
  streamController = new AbortController()
  void listenJobChanges({
    onReady: reconcile,
    onChanged: id => { if (id === jobId.value) reconcile() },
    onResync: reconcile,
  }, streamController.signal)
}
function onVisible() {
  if (!document.hidden) reconcile()
}
watch(jobId, () => { activityFilter.value = { kind: 'all' }; selectedActivityId.value = null; void load(true) }, { immediate: true })
watch(() => auth.adminToken, startStream)
onMounted(() => {
  mounted = true
  startStream()
  document.addEventListener('visibilitychange', onVisible)
  fallbackTimer = setInterval(() => {
    if (!document.hidden && job.value && !isTerminal(job.value.status)) reconcile()
  }, 60_000)
})
onBeforeUnmount(() => {
  mounted = false
  controller?.abort()
  streamController?.abort()
  document.removeEventListener('visibilitychange', onVisible)
  if (refreshTimer) clearTimeout(refreshTimer)
  if (fallbackTimer) clearInterval(fallbackTimer)
})
</script>

<template>
  <section class="space-y-5">
    <DetailHeader :title="job?.id || jobId" :back-label="t('jobUi.jobs')" @back="router.push({ name: 'jobs' })">
      <template #icon><div class="grid size-10 shrink-0 place-items-center rounded-xl bg-studio-violet/10 text-studio-violet"><Workflow class="size-5" /></div></template>
      <template #details><span class="line-clamp-2">{{ job?.goal || jobId }}</span></template>
      <template #actions>
        <Badge v-if="job" :variant="tone(job.status)">{{ statusLabel(job.status) }}</Badge>
        <Button variant="outline" :disabled="refreshing || loading || busy" @click="load()"><RefreshCw class="size-4" :class="{ 'animate-spin': refreshing }" />{{ t('jobUi.refresh') }}</Button>
        <Button v-if="job && !isTerminal(job.status) && job.status !== 'cancelling'" variant="outline" :disabled="busy" @click="cancel"><Square class="size-4" />{{ t('jobUi.cancelJob') }}</Button>
        <Button v-if="canRetry" :disabled="busy" @click="retryOpen = true"><RotateCcw class="size-4" />{{ t('jobUi.retryJob') }}</Button>
      </template>
    </DetailHeader>

    <div v-if="error" role="alert" class="rounded-xl border border-danger/30 bg-danger/5 p-4 text-sm text-danger">{{ error }}</div>
    <p v-if="notice" role="status" class="rounded-xl border border-success/30 bg-success/5 p-3 text-sm text-success-foreground">{{ notice }}</p>
    <div v-if="loading" class="studio-panel p-8 text-center text-sm text-muted-foreground">{{ t('common.loading') }}</div>

    <template v-if="job && !loading">
      <div v-if="job.error_code || unknownOutcome" role="alert" class="rounded-xl border border-danger/30 bg-danger/5 p-4">
        <div class="flex items-center gap-2 font-semibold text-danger"><AlertTriangle class="size-4" />{{ t('jobUi.executionWarning') }}</div>
        <p v-if="job.error_code" class="mt-2 break-all font-mono text-xs text-danger">{{ job.error_code }}</p>
        <p v-if="unknownOutcome" class="mt-2 text-sm text-danger">{{ t('jobUi.unknownOutcomeWarning') }}</p>
      </div>
      <div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <section class="studio-panel p-4">
          <div class="flex items-center justify-between text-xs text-muted-foreground"><span>{{ t('jobUi.timelineAgentRuns') }}</span><Layers class="size-4 text-studio-violet" /></div>
          <p class="mt-2 text-2xl font-bold tracking-tight">{{ job.agent_runs ? agentRuns.length : '—' }}</p>
          <p class="mt-1 text-xs text-muted-foreground">{{ job.agent_runs ? (agentRuns.length ? '1 root · ' + Math.max(0, agentRuns.length - 1) + ' sub-agents' : t('jobUi.noAgentRunsYet')) : t('jobUi.legacyNoAgents') }}</p>
        </section>
        <section class="studio-panel p-4">
          <div class="flex items-center justify-between text-xs text-muted-foreground"><span>{{ t('jobUi.llmRuns') }}</span><BrainCircuit class="size-4 text-studio-violet" /></div>
          <p class="mt-2 text-2xl font-bold tracking-tight">{{ llmRuns.length }}</p>
          <p class="mt-1 text-xs text-muted-foreground">{{ activeLlm }} {{ t('jobUi.llmRunning') }} · {{ llmRuns.length - activeLlm }} {{ t('jobUi.timelineObserved') }}</p>
        </section>
        <section class="studio-panel p-4">
          <div class="flex items-center justify-between text-xs text-muted-foreground"><span>{{ t('jobUi.timelinePluginCalls') }}</span><Puzzle class="size-4 text-studio-violet" /></div>
          <p class="mt-2 text-2xl font-bold tracking-tight">{{ pluginCalls }}</p>
          <p class="mt-1 text-xs text-muted-foreground">{{ t('jobUi.timelinePersistedSteps') }}</p>
        </section>
        <section class="studio-panel p-4">
          <div class="flex items-center justify-between text-xs text-muted-foreground"><span>{{ t('jobUi.timelineTotalTokens') }}</span><Cpu class="size-4 text-studio-violet" /></div>
          <p class="mt-2 text-2xl font-bold tracking-tight">{{ tokenUsage.known ? tokenUsage.tokens.toLocaleString() : '—' }}</p>
          <p class="mt-1 text-xs text-muted-foreground">{{ tokenUsage.known }} / {{ tokenUsage.total }} {{ t('jobUi.llmUsageAvailable') }}<span v-if="tokenUsage.known < tokenUsage.total"> · {{ t('jobUi.timelinePartial') }}</span></p>
        </section>
      </div>
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 class="flex items-center gap-2 font-semibold"><Activity class="size-4 text-studio-violet" />{{ t('jobUi.executionJourney') }}</h2>
          <p class="mt-1 text-xs text-muted-foreground">{{ t('jobUi.executionJourneyHint') }}</p>
        </div>
        <span class="rounded-md border px-2 py-1 text-xs text-muted-foreground">{{ t('jobUi.timelineReadOnly') }}</span>
      </div>
      <p v-if="eventsError" role="alert" class="text-xs text-danger">{{ eventsError }}</p>
      <div class="grid items-start gap-4 xl:grid-cols-[minmax(0,1.7fr)_minmax(290px,.85fr)]">
        <JobUnifiedTimeline
          :activities="filteredActivity"
          :selected-id="selectedActivityId"
          :filter="activityFilter"
          :roles="roleFilters"
          @select="selectedActivityId = $event"
          @filter="activityFilter = $event"
        />
        <aside class="min-w-0 space-y-4">
          <section class="studio-panel p-4">
            <div class="mb-3 flex items-center justify-between gap-2">
              <h3 class="flex items-center gap-2 text-sm font-semibold"><Workflow class="size-4 text-studio-violet" />{{ t('jobUi.activeAgents') }}</h3>
              <span class="text-xs text-muted-foreground">{{ agentRuns.length || '—' }}</span>
            </div>
            <p v-if="!agentRuns.length" class="text-xs text-muted-foreground">{{ job.execution_mode === 'hierarchical_v2' ? t('jobUi.noAgentRunsYet') : t('jobUi.legacyNoAgents') }}</p>
            <div v-for="run in agentRuns" :key="run.agent_run_id" class="border-t py-1">
              <button type="button" class="flex w-full items-center gap-2 rounded-md p-2 text-left transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" :class="activityFilter.kind === 'agent' && activityFilter.id === run.agent_run_id ? 'bg-primary/10' : ''" :aria-pressed="activityFilter.kind === 'agent' && activityFilter.id === run.agent_run_id" @click="filterAgent(run.agent_run_id)">
                <span class="grid size-8 shrink-0 place-items-center rounded-lg bg-primary/10 text-studio-violet"><Workflow class="size-4" /></span>
                <span class="min-w-0 flex-1">
                  <strong class="block text-xs">{{ roleName(run.role) }}</strong>
                  <span class="block truncate text-[10px] text-muted-foreground">{{ run.llm_rounds }} {{ t('jobUi.timelineRounds') }} · {{ run.tool_calls }} {{ t('jobUi.timelineTools') }}</span>
                </span>
                <span class="text-[10px] capitalize" :class="run.status === 'succeeded' ? 'text-success' : run.status === 'failed' ? 'text-danger' : 'text-studio-cyan'">{{ run.status }}</span>
              </button>
            </div>
          </section>
          <JobActivityInspector :activity="selectedActivity" />
          <section class="studio-panel p-4">
            <h3 class="mb-3 flex items-center gap-2 text-sm font-semibold"><FileText class="size-4 text-studio-violet" />{{ t('jobUi.sourceCoverage') }}</h3>
            <template v-if="coverage && coverage.required_sources_count > 0">
              <div class="flex items-center justify-between gap-3 text-xs">
                <strong>{{ coverage.verified_sources_count }} / {{ coverage.required_sources_count }} {{ t('jobUi.timelineSourcesVerified') }}</strong>
                <span class="text-muted-foreground">{{ job.status === 'running' ? t('jobUi.llmRunning') : sourcePercentage === 100 ? t('jobUi.llmCompleted') : t('jobUi.timelinePartial') }}</span>
              </div>
              <div class="mt-3 h-1.5 overflow-hidden rounded-full bg-muted"><div class="h-full rounded-full bg-primary" :style="{ width: sourcePercentage + '%' }" /></div>
              <p class="mt-2 text-xs text-muted-foreground">{{ coverage.verified_capabilities_count }} / {{ coverage.required_capabilities_count }} {{ t('jobUi.timelineCapabilities') }}</p>
            </template>
            <p v-else-if="coverage" class="text-xs text-muted-foreground">{{ t('jobUi.goalOnlyUnverified') }}</p>
            <p v-else class="text-xs text-muted-foreground">{{ t('jobUi.coverageUnavailable') }}</p>
          </section>
        </aside>
      </div>
      <details class="studio-panel p-4">
        <summary class="cursor-pointer text-sm font-semibold text-studio-violet">{{ t('jobUi.technicalDetails') }}</summary>
        <div class="mt-4 grid gap-4 xl:grid-cols-[minmax(0,1.45fr)_minmax(280px,1fr)]">
        <div class="min-w-0 space-y-4">
          <section class="studio-panel p-5">
            <div class="mb-4 flex items-center justify-between gap-2">
              <h2 class="flex items-center gap-2 font-semibold"><Workflow class="size-4 text-studio-violet" />{{ t('jobUi.steps') }}</h2>
              <span class="text-xs text-muted-foreground">{{ t('jobUi.planRevision') }} {{ job.plan_revision }} · {{ job.steps.length }} steps</span>
            </div>
            <p v-if="!job.steps.length" class="text-sm text-muted-foreground">{{ t('jobUi.noSteps') }}</p>
            <ol v-else class="space-y-0">
              <li v-for="(step, index) in job.steps" :key="step.step_key" class="relative flex gap-3 pb-6 last:pb-0">
                <div v-if="index < job.steps.length - 1" class="absolute bottom-0 left-3 top-8 w-px bg-border" aria-hidden="true" />
                <span class="relative z-10 flex size-7 shrink-0 items-center justify-center rounded-full border bg-muted text-xs font-semibold" :class="step.status === 'failed' ? 'text-danger' : step.status === 'succeeded' ? 'text-success' : 'text-studio-violet'">
                  {{ index + 1 }}
                </span>
                <div class="min-w-0 flex-1">
                  <div class="flex flex-wrap items-center justify-between gap-2">
                    <span class="break-all text-sm font-semibold">{{ step.step_key }}</span>
                    <Badge :variant="stepTone(step.status)">{{ step.status }}</Badge>
                  </div>
                  <p class="mt-1 whitespace-pre-wrap text-sm text-muted-foreground">{{ step.instruction }}</p>
                  <div v-if="step.plugin_id" class="mt-2 text-xs">
                    <button class="break-all text-studio-violet hover:underline" type="button" @click="router.push({ name: 'plugin-detail', params: { id: step.plugin_id } })">{{ step.plugin_id }}</button>
                    <span class="text-muted-foreground"> · {{ step.operation || '—' }}</span>
                  </div>
                  <p v-if="step.dependencies?.length" class="mt-1 text-xs text-muted-foreground">{{ t('jobUi.dependencies') }}: {{ step.dependencies.join(', ') }}</p>
                  <p v-if="step.attempt_state || step.error_code" class="mt-1 break-all text-xs" :class="step.error_code ? 'text-danger' : 'text-muted-foreground'">
                    {{ step.attempt_state }} {{ step.error_code ? '· ' + step.error_code : '' }}
                  </p>
                </div>
              </li>
            </ol>
          </section>
          <section class="studio-panel p-5">
            <h2 class="mb-4 flex items-center gap-2 font-semibold"><FileText class="size-4 text-studio-violet" />{{ t('jobUi.artifacts') }}</h2>
            <p v-if="!job.artifacts.length" class="text-sm text-muted-foreground">{{ t('jobUi.noArtifacts') }}</p>
            <div v-for="artifact in job.artifacts" :key="artifact.id" class="flex items-center justify-between gap-3 border-t py-3">
              <div class="min-w-0">
                <p class="truncate font-mono text-xs">{{ artifact.id }}</p>
                <p class="mt-1 text-xs text-muted-foreground">{{ artifact.media_type }} · {{ artifact.size_bytes }} B</p>
              </div>
              <Button variant="outline" size="sm" @click="downloadArtifact(artifact.id)"><Download class="size-4" />{{ t('jobUi.download') }}</Button>
            </div>
          </section>
          <section v-if="summary(job.result)" class="studio-panel p-5">
            <h2 class="mb-3 font-semibold">{{ t('jobUi.result') }}</h2>
            <pre class="max-h-80 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted/40 p-3 text-xs leading-relaxed">{{ summary(job.result) }}</pre>
          </section>
        </div>

        <aside class="min-w-0 space-y-4">
          <section class="studio-panel p-5">
            <h2 class="mb-4 font-semibold">{{ t('jobUi.executionContext') }}</h2>
            <div class="divide-y text-sm">
              <div class="flex justify-between gap-3 py-3"><span class="text-muted-foreground">{{ t('jobUi.requester') }}</span><span class="max-w-[65%] break-all text-right">{{ job.requester_scope }}</span></div>
              <div class="flex justify-between gap-3 py-3"><span class="text-muted-foreground">{{ t('jobUi.createdAt') }}</span><span class="text-right"><Clock3 class="mr-1 inline size-3.5" />{{ date(job.created_at) }}</span></div>
              <div class="flex justify-between gap-3 py-3"><span class="text-muted-foreground">{{ t('jobUi.status') }}</span><Badge :variant="tone(job.status)">{{ statusLabel(job.status) }}</Badge></div>
              <div class="flex justify-between gap-3 py-3"><span class="text-muted-foreground">{{ t('jobUi.planRevision') }}</span><span>{{ job.plan_revision }}</span></div>
              <div v-if="job.retry_of_job_id" class="flex flex-wrap justify-between gap-3 py-3">
                <span class="text-muted-foreground">{{ t('jobUi.retryOf') }}</span>
                <button type="button" class="break-all text-studio-violet hover:underline" @click="router.push({ name: 'job-detail', params: { jobId: job.retry_of_job_id } })">{{ job.retry_of_job_id }}</button>
              </div>
            </div>
            <h3 class="mt-5 text-sm font-semibold">{{ t('jobUi.budgetUsed') }}</h3>
            <pre class="mt-2 max-h-44 overflow-auto whitespace-pre-wrap break-all rounded-lg bg-muted/40 p-3 font-mono text-xs">{{ budget() }}</pre>
          </section>
          <section class="studio-panel p-5">
            <div class="mb-4 flex items-center justify-between gap-2">
              <h2 class="font-semibold">{{ t('jobUi.executionTimeline') }}</h2>
              <span class="text-xs text-muted-foreground">{{ events.length }} events</span>
            </div>
            <p v-if="eventsError" role="alert" class="mb-3 text-xs text-danger">{{ eventsError }}</p>
            <p v-if="!events.length" class="text-sm text-muted-foreground">{{ t('jobUi.noEvents') }}</p>
            <div v-if="planningEvents.length" class="mb-4">
              <h3 class="mb-3 text-xs font-semibold uppercase tracking-wide text-muted-foreground">{{ t('jobUi.planningAttempts') }}</h3>
              <div v-for="event in planningEvents" :key="event.sequence" class="mb-3 border-l-2 border-studio-violet pl-3 text-xs">
                <p class="font-medium">{{ event.event_type }} · #{{ event.sequence }}</p>
                <p class="mt-1 text-muted-foreground">{{ date(event.created_at) }}</p>
                <p v-if="event.payload.code || event.payload.message" class="mt-1 break-words text-danger">{{ event.payload.code || event.payload.message }}</p>
              </div>
            </div>
            <div v-if="executionEvents.length">
              <h3 class="mb-3 text-xs font-semibold uppercase tracking-wide text-muted-foreground">{{ t('jobUi.execution') }}</h3>
              <div v-for="event in executionEvents" :key="event.sequence" class="mb-3 border-l-2 border-border pl-3 text-xs">
                <p class="font-medium">{{ event.event_type }}<span v-if="event.step_key"> · {{ event.step_key }}</span></p>
                <p class="mt-1 text-muted-foreground">{{ date(event.created_at) }} · {{ event.payload.status || event.payload.attempt_state || '' }}</p>
                <p v-if="event.payload.error_code" class="mt-1 text-danger">{{ event.payload.error_code }}</p>
              </div>
            </div>
          </section>
        </aside>
        </div>
      </details>
    </template>

    <ConfirmDialog v-model="retryOpen" :title="t('jobUi.retryJob')" :confirm-label="t('jobUi.retryJob')" @confirm="retry">
      <p>{{ t('jobUi.retryConfirm') }}</p>
      <p v-if="unknownOutcome" class="mt-2 text-danger">{{ t('jobUi.unknownOutcomeWarning') }}</p>
    </ConfirmDialog>
  </section>
</template>

<script setup lang="ts">
import { Activity, AlertTriangle, BrainCircuit, Cpu, FileText, Layers, Puzzle, RefreshCw, RotateCcw, Square, Trash2, Workflow } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { agentJobsApi } from '@/api/agent-jobs'
import { listenJobChanges } from '@/api/job-updates'
import { useAuthStore } from '@/stores/auth'
import { formatApiError, isApiError } from '@/api/errors'
import type { AdminJob, JobEvent, JobStatus } from '@/api/types/agent-jobs'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import JobUnifiedTimeline from '@/components/jobs/JobUnifiedTimeline.vue'
import JobActivityInspector from '@/components/jobs/JobActivityInspector.vue'
import JobArtifactGallery from '@/components/jobs/artifacts/JobArtifactGallery.vue'
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
const deleteOpen = ref(false)
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
const selectedAgent = computed(() => agentRuns.value.find(run => run.agent_run_id === selectedActivity.value?.agentRunId) ?? null)
const selectedSteps = computed(() => {
  const activity = selectedActivity.value
  if (activity?.step) return [activity.step]
  return activity?.agentRunId ? job.value?.steps.filter(step => step.agent_run_id === activity.agentRunId) ?? [] : []
})
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
const statusLabel = (status: JobStatus) => t(('jobUi.jobStatus.' + status) as 'jobUi.jobStatus.queued')
function summary(result: AdminJob['result']) {
  if (!result) return ''
  return typeof result.summary === 'string' ? result.summary : JSON.stringify(result, null, 2)
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
async function deleteJob() {
  if (!job.value || !isTerminal(job.value.status) || busy.value) return
  busy.value = true
  error.value = ''
  try {
    await agentJobsApi.deleteJob(job.value.id)
    await router.push({ name: 'jobs' })
  } catch (cause) {
    error.value = isApiError(cause) && cause.code === 'job_delete_conflict'
      ? t('jobUi.deleteConflict') : formatApiError(cause)
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
        <Button v-if="job && isTerminal(job.status)" variant="outline" class="text-danger" :disabled="busy || loading || refreshing" @click="deleteOpen = true"><Trash2 class="size-4" />{{ t('jobUi.deleteJob') }}</Button>
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
            <p v-if="!agentRuns.length" class="text-xs text-muted-foreground">{{ t('jobUi.noAgentRunsYet') }}</p>
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
            <div v-if="selectedActivity" :key="selectedActivity.id" class="mt-4 space-y-3 border-t pt-3 text-xs">
              <h4 class="break-words font-semibold">{{ selectedActivity.title }}</h4>
              <details v-if="selectedAgent?.goal_excerpt">
                <summary class="cursor-pointer font-semibold">{{ t('jobUi.goal') }}</summary>
                <p class="mt-1 whitespace-pre-wrap break-words">{{ selectedAgent.goal_excerpt }}</p>
              </details>
              <details v-for="step in selectedSteps" :key="step.step_key" class="space-y-2 border-t pt-3">
                <summary class="cursor-pointer break-words font-semibold">{{ step.instruction }}</summary>
                <p class="break-all font-mono text-muted-foreground">{{ step.step_key }}</p>
                <h5 class="text-muted-foreground">{{ t('jobUi.toolArguments') }}</h5>
                <pre class="max-h-60 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted/40 p-3">{{ JSON.stringify(step.arguments, null, 2) }}</pre>
                <h5 class="text-muted-foreground">{{ t('jobUi.result') }}</h5>
                <pre class="max-h-80 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted/40 p-3">{{ step.result == null ? '—' : JSON.stringify(step.result, null, 2) }}</pre>
              </details>
              <details v-if="selectedAgent?.result">
                <summary class="cursor-pointer font-semibold">{{ t('jobUi.agentResult') }}</summary>
                <pre class="mt-2 max-h-80 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted/40 p-3">{{ JSON.stringify(selectedAgent.result, null, 2) }}</pre>
              </details>
              <p v-if="!selectedSteps.length && !selectedAgent?.result" class="text-muted-foreground">{{ t('jobUi.noSourceDetails') }}</p>
            </div>
          </section>
          <section class="studio-panel p-5">
            <h2 class="mb-4 flex items-center gap-2 font-semibold"><FileText class="size-4 text-studio-violet" />{{ t('jobUi.artifacts') }}</h2>
            <JobArtifactGallery :job="job" />
          </section>
        </aside>
      </div>
      <section class="studio-panel p-5">
        <h2 class="mb-3 font-semibold">{{ t('jobUi.result') }}</h2>
        <pre v-if="summary(job.result)" class="max-h-80 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted/40 p-3 text-xs leading-relaxed">{{ summary(job.result) }}</pre>
        <p v-else class="text-sm text-muted-foreground">—</p>
      </section>
    </template>

    <ConfirmDialog v-model="retryOpen" :title="t('jobUi.retryJob')" :confirm-label="t('jobUi.retryJob')" @confirm="retry">
      <p>{{ t('jobUi.retryConfirm') }}</p>
      <p v-if="unknownOutcome" class="mt-2 text-danger">{{ t('jobUi.unknownOutcomeWarning') }}</p>
    </ConfirmDialog>
    <ConfirmDialog v-model="deleteOpen" :title="t('jobUi.deleteJob')" :confirm-label="t('jobUi.deleteJob')" tone="danger" @confirm="deleteJob">
      <p>{{ t('jobUi.deleteConfirm') }}</p>
    </ConfirmDialog>
  </section>
</template>

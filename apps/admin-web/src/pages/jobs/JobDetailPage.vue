<script setup lang="ts">
import { Activity, AlertTriangle, RefreshCw, RotateCcw, Square, Trash2, Workflow } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { agentJobsApi } from '@/api/agent-jobs'
import { listenJobChanges } from '@/api/job-updates'
import { useAuthStore } from '@/stores/auth'
import { formatApiError, isApiError } from '@/api/errors'
import type { AdminJob, JobEvent, JobStatus } from '@/api/types/agent-jobs'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import JobUnifiedTimeline, { type JobCanvasStats } from '@/components/jobs/JobUnifiedTimeline.vue'
import KnowledgeJobState from '@/components/jobs/KnowledgeJobState.vue'
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
const canRetry = computed(() => job.value && job.value.execution_mode !== 'knowledge_ingest_v3' && ['failed', 'cancelled'].includes(job.value.status))
const unknownOutcome = computed(() => job.value?.steps.some(step => step.attempt_state === 'unknown_outcome') ?? false)

const activityFilter = ref<ActivityFilter>({ kind: 'all' })
const selectedActivityId = ref<string | null>(null)
const llmRuns = computed(() => job.value ? projectLlmRuns(events.value, job.value.status, job.value.agent_runs) : [])
const tokenUsage = computed(() => llmTokenSummary(llmRuns.value))
const activeLlm = computed(() => llmRuns.value.filter(run => run.status === 'running').length)
const pluginCalls = computed(() => job.value?.steps.filter(step => step.plugin_id && (step.attempt_state || step.status !== 'pending')).length ?? 0)
const agentRuns = computed(() => [...(job.value?.agent_runs ?? [])].sort((a, b) => a.run_index - b.run_index))
const canvasStats = computed<JobCanvasStats | undefined>(() =>
  job.value
    ? {
        agentRunsCount: job.value.agent_runs ? agentRuns.value.length : null,
        llmRunsCount: llmRuns.value.length,
        activeLlmCount: activeLlm.value,
        pluginCallsCount: pluginCalls.value,
        tokenUsage: tokenUsage.value,
      }
    : undefined,
)
const roleFilters = computed(() => [...new Set(agentRuns.value.map(run => run.role))])
const allActivity = computed(() => job.value ? projectUnifiedActivity(job.value, events.value) : [])
const filteredActivity = computed(() => visibleActivities(allActivity.value, activityFilter.value))
watch(filteredActivity, rows => {
  if (!rows.some(item => item.id === selectedActivityId.value)) selectedActivityId.value = rows[0]?.id ?? null
}, { immediate: true })

const tone = (status: JobStatus): 'success' | 'danger' | 'secondary' | 'outline' =>
  status === 'succeeded' ? 'success' : status === 'failed' ? 'danger' :
    status === 'partial_success' || status === 'cancelled' ? 'outline' : 'secondary'
const statusLabel = (status: JobStatus) => t(('jobUi.jobStatus.' + status) as 'jobUi.jobStatus.queued')
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
        <Badge v-if="job?.execution_mode === 'knowledge_ingest_v3'" variant="outline">{{ t('jobUi.knowledge') }}</Badge>
        <Badge v-if="job" :variant="tone(job.status)">{{ statusLabel(job.status) }}</Badge>
        <Button variant="outline" :disabled="refreshing || loading || busy" @click="load()"><RefreshCw class="size-4" :class="{ 'animate-spin': refreshing }" />{{ t('jobUi.refresh') }}</Button>
        <Button v-if="job && !isTerminal(job.status) && job.status !== 'cancelling'" variant="outline" :disabled="busy" @click="cancel"><Square class="size-4" />{{ t('jobUi.cancelJob') }}</Button>
        <Button v-if="canRetry" :disabled="busy" @click="retryOpen = true"><RotateCcw class="size-4" />{{ t('jobUi.retryJob') }}</Button>
        <Button v-if="job && isTerminal(job.status)" variant="outline" class="text-danger" :disabled="busy || loading || refreshing" @click="deleteOpen = true"><Trash2 class="size-4" />{{ t('jobUi.deleteJob') }}</Button>
      </template>
    </DetailHeader>

    <div v-if="error" role="alert" class="rounded-xl border border-danger/30 bg-danger/5 p-4 text-sm text-danger">{{ error }}</div>
    <p v-if="notice" role="status" class="rounded-xl border border-success/30 bg-success/5 p-3 text-sm text-success-foreground">{{ notice }}</p>
    <div
      v-if="loading"
      data-job-detail-skeleton
      role="status"
      aria-busy="true"
      :aria-label="t('common.loading')"
      class="space-y-5"
    >
      <div class="card-shimmer flex flex-wrap items-center justify-between gap-3 rounded-xl py-1" aria-hidden="true">
        <div class="space-y-2">
          <div class="h-5 w-44 rounded bg-muted/60" />
          <div class="h-3 w-64 max-w-full rounded bg-muted/40" />
        </div>
        <div class="h-6 w-24 rounded-md bg-muted/40" />
      </div>

      <div class="studio-panel card-shimmer space-y-4 p-4 sm:p-5" aria-hidden="true">
        <div class="flex flex-wrap gap-2">
          <div v-for="pill in 4" :key="pill" class="h-8 w-24 rounded-lg bg-muted/50" />
        </div>
        <div class="grid gap-2 sm:grid-cols-2 xl:grid-cols-4">
          <div v-for="slot in 4" :key="slot" class="space-y-2 rounded-lg border border-border/50 bg-muted/20 p-2.5">
            <div class="flex items-center justify-between">
              <div class="h-3 w-20 rounded bg-muted/50" />
              <div class="size-3.5 rounded bg-muted/60" />
            </div>
            <div class="h-5 w-12 rounded bg-muted/60" />
            <div class="h-2.5 w-28 rounded bg-muted/40" />
          </div>
        </div>
        <div class="grid gap-4 lg:grid-cols-[minmax(0,1fr)_320px]">
          <div class="space-y-3">
            <div v-for="step in 4" :key="step" class="h-20 rounded-xl bg-muted/40" />
          </div>
          <div class="hidden h-80 rounded-xl bg-muted/40 lg:block" />
        </div>
      </div>
    </div>

    <template v-if="job && !loading">
      <KnowledgeJobState v-if="job.execution_mode === 'knowledge_ingest_v3'" :job="job" @changed="load()" />
      <div v-if="job.error_code || unknownOutcome" role="alert" class="rounded-xl border border-danger/30 bg-danger/5 p-4">
        <div class="flex items-center gap-2 font-semibold text-danger"><AlertTriangle class="size-4" />{{ t('jobUi.executionWarning') }}</div>
        <p v-if="job.error_code" class="mt-2 break-all font-mono text-xs text-danger">{{ job.error_code }}</p>
        <p v-if="unknownOutcome" class="mt-2 text-sm text-danger">{{ t('jobUi.unknownOutcomeWarning') }}</p>
      </div>
      <div class="sectionbar flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 class="flex flex-wrap items-center gap-2 font-semibold">
            <Activity class="size-4 text-studio-violet" />{{ t('jobUi.executionJourney') }}
          </h2>
          <p class="mt-1 text-xs text-muted-foreground">{{ t('jobUi.executionJourneyHint') }}</p>
        </div>
        <span class="rounded-md border px-2 py-1 text-xs text-muted-foreground">{{ t('jobUi.timelineReadOnly') }}</span>
      </div>
      <p v-if="eventsError" role="alert" class="text-xs text-danger">{{ eventsError }}</p>
      <JobUnifiedTimeline
        :job-id="job.id"
        :activities="filteredActivity"
        :selected-id="selectedActivityId"
        :filter="activityFilter"
        :roles="roleFilters"
        :stats="canvasStats"
        @select="selectedActivityId = $event"
        @filter="activityFilter = $event"
      />
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

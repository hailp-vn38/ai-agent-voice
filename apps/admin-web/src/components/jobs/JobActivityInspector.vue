<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { Cpu, Download, Eye, File, X } from '@lucide/vue'
import { agentJobsApi } from '@/api/agent-jobs'
import { formatApiError } from '@/api/errors'
import type { JobArtifact, JobEvent } from '@/api/types/agent-jobs'
import { jobResultReport } from '@/domain/jobs/result-report'
import type { ActivityKind, UnifiedActivity } from '@/domain/jobs/unified-activity'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import ArtifactPreviewDialog, { artifactPreview } from './artifacts/ArtifactPreviewDialog.vue'

const props = withDefaults(
  defineProps<{
    jobId?: string
    activity: UnifiedActivity | null
    open?: boolean
  }>(),
  { open: true },
)
const emit = defineEmits<{
  close: []
}>()
const { t } = useI18n()
const run = computed(() => props.activity?.llm ?? null)
const step = computed(() => props.activity?.step ?? null)
const stepAssistantMessage = computed(() => props.activity?.assistantMessage ?? null)
const agentMessages = computed(() =>
  (props.activity?.agentRun?.messages ?? []).filter(m => m.text && m.text.trim().length > 0),
)
const activityArtifacts = computed(() => props.activity?.artifacts ?? [])
const report = computed(() => jobResultReport(props.activity?.jobResult))
const selectedArtifact = ref<JobArtifact | null>(null)
const artifactError = ref('')
const downloadingId = ref<string | null>(null)

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && props.open) {
    if (selectedArtifact.value) {
      selectedArtifact.value = null
      return
    }
    emit('close')
  }
}

onMounted(() => window.addEventListener('keydown', onKeydown))
onBeforeUnmount(() => window.removeEventListener('keydown', onKeydown))

function canPreview(item: JobArtifact): boolean {
  const preview = artifactPreview(item)
  return Boolean(props.jobId) && preview.kind !== 'download' && item.size_bytes >= 0 && item.size_bytes <= preview.limit
}

async function downloadArtifact(item: JobArtifact) {
  if (!props.jobId || downloadingId.value) return
  downloadingId.value = item.id
  artifactError.value = ''
  try {
    const data = await agentJobsApi.artifact(props.jobId, item.id)
    const url = URL.createObjectURL(data)
    const link = document.createElement('a')
    link.href = url
    link.download = item.filename || item.name || item.id
    link.click()
    window.setTimeout(() => URL.revokeObjectURL(url), 1000)
  } catch (cause) {
    artifactError.value = formatApiError(cause)
  } finally {
    downloadingId.value = null
  }
}
const pendingStage = computed(() => {
  if (run.value?.status !== 'running') return null
  switch (run.value.events.at(-1)?.event_type) {
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
function kindBadge(kind: ActivityKind): string {
  switch (kind) {
    case 'llm':
      return '✳ LLM RUN'
    case 'plugin':
      return '⬡ PLUGIN STEP'
    case 'fork':
      return '⑂ DELEGATION'
    case 'join':
      return '⑃ JOIN'
    case 'job':
      return '✓ JOB'
    case 'result':
      return '📄 RESULT'
    case 'media':
      return '🖼️ MEDIA'
    case 'agent':
      return '◉ AGENT RUN'
    case 'plan':
      return '◌ PLAN'
  }
}
function duration(ms: number | null): string {
  return ms === null ? '—' : (ms / 1000).toFixed(2) + ' s'
}
function stageLabel(event: JobEvent): string {
  switch (event.event_type) {
    case 'llm_run_started':
    case 'agent_run_started':
      return t('jobUi.llmStarted')
    case 'llm_provider_acquired':
      return t('jobUi.llmProviderAcquired')
    case 'llm_request_started':
      return t('jobUi.llmDispatched')
    case 'llm_response_received':
      return t('jobUi.llmReceived')
    case 'llm_run_finished':
    case 'agent_run_finished':
      return t('jobUi.llmFinished')
    case 'plan_rejected':
      return t('jobUi.llmValidationFailed')
    case 'agent_decision_rejected':
      return t('jobUi.llmValidationFailed')
    case 'agent_decision_accepted':
      return 'Decision accepted'
    default:
      return event.event_type
  }
}
function safeDetail(value: unknown): string {
  return typeof value === 'string' || typeof value === 'number' ? String(value) : '—'
}
function stageNote(event: JobEvent): string {
  if (event.event_type === 'llm_provider_acquired') return safeDetail(event.payload.provider_wait_ms) + ' ms'
  if (event.event_type === 'llm_request_started') return safeDetail(event.payload.capacity_wait_ms) + ' ms'
  if (event.event_type === 'llm_response_received') return safeDetail(event.payload.request_duration_ms) + ' ms'
  if (event.event_type === 'plan_rejected') return safeDetail(event.payload.code)
  if (event.event_type === 'agent_decision_rejected') return safeDetail(event.payload.validation_code)
  if (event.event_type === 'llm_run_finished') return safeDetail(event.payload.error_code ?? event.payload.validation_code)
  return ''
}
</script>

<template>
  <div
    v-if="props.open"
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/65 p-4 backdrop-blur-[2px] sm:p-6"
    role="presentation"
    data-activity-modal-backdrop
    @click.self="emit('close')"
  >
    <section
      :key="activity?.id"
      role="dialog"
      aria-modal="true"
      :aria-label="activity?.title || t('jobUi.eventInspector')"
      data-activity-modal
      class="studio-panel inspector relative z-10 flex max-h-[88vh] w-full max-w-2xl flex-col overflow-hidden border bg-background shadow-2xl"
    >
      <header class="surface-head flex shrink-0 items-center justify-between gap-2 border-b px-5 py-3.5">
        <h3 class="inline-flex items-center gap-2 text-xs font-semibold">
          <Cpu class="size-4 text-studio-violet" />{{ t('jobUi.eventInspector') }}
        </h3>
        <div class="flex items-center gap-1.5">
          <Badge variant="outline" class="ml-2 text-[10px]">{{ t('jobUi.llmReadOnly') }}</Badge>
          <Button
            variant="ghost"
            size="icon"
            class="size-7"
            data-inspector-close
            :aria-label="t('common.close')"
            @click="emit('close')"
          >
            <X class="size-4" />
          </Button>
        </div>
      </header>
      <div class="ins-body overflow-y-auto p-5">
      <p v-if="!activity" class="text-xs text-muted-foreground">{{ t('jobUi.selectTimelineEvent') }}</p>
      <template v-else>
        <div class="ins-heading mb-3 flex flex-wrap items-center justify-between gap-2">
          <span class="ins-type">{{ kindBadge(activity.kind) }}</span>
          <span
            class="text-[10px] font-semibold capitalize"
            :class="
              activity.status === 'completed'
                ? 'text-success'
                : activity.status === 'failed' || activity.status === 'unknown'
                  ? 'text-danger'
                  : 'text-studio-cyan'
            "
          >
            ● {{ activity.status }}
          </span>
        </div>
        <strong class="ins-name block break-words text-base font-bold">{{ activity.title }}</strong>
        <p class="ins-sub mb-3 mt-1 break-words text-xs text-muted-foreground">{{ activity.note }}</p>

        <template v-if="run">
          <dl class="divide-y divide-border/60 text-xs">
            <div class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.llmProvider') }}</dt>
              <dd class="max-w-[65%] break-all text-right font-semibold">{{ run.provider }}</dd>
            </div>
            <div class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.llmModel') }}</dt>
              <dd class="max-w-[65%] break-all text-right font-semibold">{{ run.model ?? '—' }}</dd>
            </div>
            <div class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.llmAdapter') }}</dt>
              <dd class="max-w-[65%] break-all text-right font-semibold">{{ run.adapter ?? '—' }}</dd>
            </div>
            <div class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.llmRole') }}</dt>
              <dd class="text-right font-semibold">{{ run.role }}</dd>
            </div>
            <div class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.llmTransport') }}</dt>
              <dd class="text-right font-semibold">{{ run.transport ?? '—' }}</dd>
            </div>
            <div class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.llmSource') }}</dt>
              <dd class="text-right font-semibold">{{ run.providerSource }}</dd>
            </div>
            <div class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.llmDuration') }}</dt>
              <dd class="text-right font-semibold">{{ duration(run.totalDurationMs ?? run.requestDurationMs) }}</dd>
            </div>
            <div class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.llmTokens') }}</dt>
              <dd class="text-right font-semibold">{{ run.inputTokens ?? '—' }} / {{ run.outputTokens ?? '—' }}</dd>
            </div>
            <div class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.llmRunId') }}</dt>
              <dd class="max-w-[65%] break-all text-right font-mono text-[11px]">{{ run.id }}</dd>
            </div>
            <div v-if="run.events[0]?.payload.turn_id" class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.agentTurnId') }}</dt>
              <dd class="max-w-[65%] break-all text-right font-mono text-[11px]">{{ run.events[0]?.payload.turn_id }}</dd>
            </div>
            <div v-if="activity.agentRunId" class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.agentRunId') }}</dt>
              <dd class="max-w-[65%] break-all text-right font-mono text-[11px]">{{ activity.agentRunId }}</dd>
            </div>
          </dl>
          <details v-if="stepAssistantMessage" class="mt-3 text-xs" open>
            <summary class="cursor-pointer font-medium">{{ t('jobUi.agentMessage') }}</summary>
            <p class="mt-2 whitespace-pre-wrap break-words rounded-lg bg-muted p-2">{{ stepAssistantMessage }}</p>
          </details>
          <p v-else-if="!agentMessages.length" class="mt-3 text-xs text-muted-foreground">{{ t('jobUi.llmContentUnavailable') }}</p>
          <div class="mt-4 border-t pt-3">
            <h4 class="mb-2 text-[10px] font-bold uppercase tracking-wide text-muted-foreground">
              ◴ {{ t('jobUi.llmTimeline') }}
            </h4>
            <div
              v-for="stage in run.events"
              :key="stage.sequence"
              class="event-line flex items-start justify-between gap-2 border-l-2 border-primary/40 py-1 pl-2.5 text-[11px]"
            >
              <span class="min-w-0 break-words">
                {{ stageLabel(stage) }}<span v-if="stage.payload.validation_path" class="ml-1 break-all font-mono text-danger">{{ stage.payload.validation_path }}</span>
              </span>
              <span class="shrink-0 text-muted-foreground">{{ stageNote(stage) }}</span>
            </div>
            <p v-if="pendingStage" class="mt-2 text-xs text-studio-cyan">{{ pendingStage }}</p>
            <p v-if="run.errorCode" class="mt-2 break-all rounded-lg bg-danger/5 p-2 font-mono text-xs text-danger">
              {{ run.errorCode }}
            </p>
          </div>
        </template>

        <section v-if="step" class="mt-4 border-t pt-3">
          <h4 v-if="run" class="mb-2 text-xs font-semibold">{{ t('jobUi.llmCheckpointProgress') }}</h4>
          <p v-if="step.instruction" class="mb-2 break-words text-xs font-medium">{{ step.instruction }}</p>
          <dl class="divide-y divide-border/60 text-xs">
            <div v-if="step.plugin_id" class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.plugin') }}</dt>
              <dd class="max-w-[65%] break-all text-right font-semibold">{{ step.plugin_id ?? '—' }}</dd>
            </div>
            <div v-if="step.plugin_id" class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.operation') }}</dt>
              <dd class="max-w-[65%] break-all text-right font-semibold">{{ step.operation ?? '—' }}</dd>
            </div>
            <div class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.status') }}</dt>
              <dd class="text-right font-semibold">{{ step.status }}</dd>
            </div>
            <div class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.attemptState') }}</dt>
              <dd class="text-right font-semibold">{{ step.attempt_state ?? '—' }}</dd>
            </div>
            <div class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.stepKey') }}</dt>
              <dd class="max-w-[65%] break-all text-right font-mono text-[11px]">{{ step.step_key }}</dd>
            </div>
            <div v-if="step.error_code" class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.executionWarning') }}</dt>
              <dd class="max-w-[65%] break-all text-right font-mono text-danger">{{ step.error_code }}</dd>
            </div>
          </dl>
          <template v-if="step.plugin_id">
            <details v-if="stepAssistantMessage" class="mt-3 text-xs" open>
              <summary class="cursor-pointer font-medium">{{ t('jobUi.agentMessage') }}</summary>
              <p class="mt-2 whitespace-pre-wrap break-words rounded-lg bg-muted p-2">{{ stepAssistantMessage }}</p>
            </details>
            <details class="mt-3 text-xs">
              <summary class="cursor-pointer font-medium">{{ t('jobUi.toolArguments') }}</summary>
              <pre class="ins-pre mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-lg bg-muted p-2">{{ JSON.stringify(step.arguments, null, 2) ?? '—' }}</pre>
            </details>
            <details class="mt-3 text-xs">
              <summary class="cursor-pointer font-medium">{{ t('jobUi.result') }}</summary>
              <pre class="ins-pre mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-lg bg-muted p-2">{{ JSON.stringify(step.result, null, 2) ?? '—' }}</pre>
            </details>
          </template>
          <p v-if="step.attempt_state === 'unknown_outcome'" role="alert" class="mt-3 text-xs text-danger">
            {{ t('jobUi.unknownOutcomeWarning') }}
          </p>
        </section>

        <template v-if="!run && !step">
          <dl class="divide-y divide-border/60 text-xs">
            <div v-if="activity.agentRunId" class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.agentRunId') }}</dt>
              <dd class="max-w-[65%] break-all text-right font-mono">{{ activity.agentRunId }}</dd>
            </div>
            <div v-if="activity.event?.payload.fork_group_id" class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.forkGroup') }}</dt>
              <dd class="max-w-[65%] break-all text-right font-mono">{{ safeDetail(activity.event.payload.fork_group_id) }}</dd>
            </div>
            <div class="detail-row flex items-start justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('jobUi.execution') }}</dt>
              <dd class="max-w-[65%] break-words text-right">{{ activity.note }}</dd>
            </div>
          </dl>
          <details v-if="stepAssistantMessage" class="mt-3 text-xs" open>
            <summary class="cursor-pointer font-medium">{{ t('jobUi.agentMessage') }}</summary>
            <p class="mt-2 whitespace-pre-wrap break-words rounded-lg bg-muted p-2">{{ stepAssistantMessage }}</p>
          </details>
        </template>

        <details v-if="activity.agentRun?.goal_excerpt" class="mt-3 border-t pt-3 text-xs">
          <summary class="cursor-pointer font-medium">{{ t('jobUi.goal') }}</summary>
          <p class="mt-1 whitespace-pre-wrap break-words">{{ activity.agentRun.goal_excerpt }}</p>
        </details>
        <details v-if="agentMessages.length" class="mt-3 border-t pt-3 text-xs">
          <summary class="cursor-pointer font-medium">{{ t('jobUi.agentMessages') }} ({{ agentMessages.length }})</summary>
          <div class="mt-2 space-y-2">
            <div
              v-for="msg in agentMessages"
              :key="msg.id"
              class="rounded-lg bg-muted p-2"
            >
              <div class="mb-1 flex items-center justify-between text-[10px] font-semibold uppercase text-muted-foreground">
                <span>{{ msg.role }}</span>
                <span class="font-mono">#{{ msg.sequence }}</span>
              </div>
              <p class="whitespace-pre-wrap break-words">{{ msg.text }}</p>
            </div>
          </div>
        </details>
        <details v-if="report.kind !== 'empty'" class="mt-3 border-t pt-3 text-xs" open>
          <summary class="cursor-pointer font-medium">{{ t('jobUi.result') }}</summary>
          <article
            v-if="report.kind === 'markdown'"
            class="job-report mt-2 rounded-lg bg-muted p-2.5 text-xs leading-6"
            v-html="report.html"
          />
          <pre
            v-else-if="report.kind === 'json'"
            class="ins-pre mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-lg bg-muted p-2"
          >{{ report.text }}</pre>
        </details>
        <details v-if="activityArtifacts.length" class="mt-3 border-t pt-3 text-xs" open>
          <summary class="cursor-pointer font-medium">{{ t('jobUi.artifacts') }} ({{ activityArtifacts.length }})</summary>
          <div class="mt-2 space-y-2">
            <div
              v-for="item in activityArtifacts"
              :key="item.id"
              class="flex items-center justify-between gap-2 rounded-lg bg-muted p-2"
            >
              <div class="flex min-w-0 items-center gap-2">
                <File class="size-3.5 shrink-0 text-studio-violet" />
                <div class="min-w-0">
                  <p class="truncate font-medium" :title="item.filename || item.name || item.id">
                    {{ item.filename || item.name || item.id }}
                  </p>
                  <p class="truncate text-[10px] text-muted-foreground">
                    {{ item.media_type || item.mime_type }} · {{ (item.size_bytes / 1024).toFixed(1) }} KiB
                  </p>
                </div>
              </div>
              <div v-if="props.jobId" class="flex shrink-0 items-center gap-1">
                <Button
                  variant="outline"
                  size="sm"
                  class="h-7 px-2 text-[10px]"
                  :disabled="downloadingId !== null"
                  @click="downloadArtifact(item)"
                >
                  <Download class="size-3" />{{ t('jobUi.download') }}
                </Button>
                <Button
                  v-if="canPreview(item)"
                  variant="outline"
                  size="sm"
                  class="h-7 px-2 text-[10px]"
                  data-artifact-preview
                  @click="selectedArtifact = item"
                >
                  <Eye class="size-3" />Preview
                </Button>
              </div>
            </div>
            <p v-if="artifactError" role="alert" class="text-xs text-danger">{{ artifactError }}</p>
          </div>
        </details>
        <details v-if="activity.agentRun?.result" class="mt-3 border-t pt-3 text-xs">
          <summary class="cursor-pointer font-medium">{{ t('jobUi.agentResult') }}</summary>
          <pre class="ins-pre mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-lg bg-muted p-2">{{ JSON.stringify(activity.agentRun.result, null, 2) }}</pre>
        </details>
      </template>
      </div>
      <ArtifactPreviewDialog
        v-if="selectedArtifact && props.jobId"
        :key="selectedArtifact.id"
        :job-id="props.jobId"
        :artifact="selectedArtifact"
        @close="selectedArtifact = null"
        @download="selectedArtifact && downloadArtifact(selectedArtifact)"
      />
    </section>
  </div>
</template>

<style scoped>
.ins-type {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  background: #25233f;
  color: #c5b7ff;
  border: 1px solid #51477b;
  padding: 3px 8px;
  border-radius: 6px;
  font-size: 10px;
  font-weight: 720;
}
</style>

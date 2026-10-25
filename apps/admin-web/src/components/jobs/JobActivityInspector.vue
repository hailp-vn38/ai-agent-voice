<script setup lang="ts">
import { computed } from 'vue'
import { Cpu } from '@lucide/vue'
import type { JobEvent } from '@/api/types/agent-jobs'
import type { ActivityKind, UnifiedActivity } from '@/domain/jobs/unified-activity'
import { Badge } from '@/components/ui/badge'
import { useI18n } from '@/composables/useI18n'

const props = withDefaults(
  defineProps<{
    activity: UnifiedActivity | null
    open?: boolean
    closable?: boolean
  }>(),
  { open: false, closable: false },
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
  <details :key="activity?.id" :open="props.open || undefined" class="studio-panel inspector overflow-hidden">
    <summary class="surface-head flex cursor-pointer list-none items-center justify-between gap-2 border-b px-4 py-3">
      <h3 class="inline-flex items-center gap-2 text-xs font-semibold">
        <Cpu class="size-4 text-studio-violet" />{{ t('jobUi.eventInspector') }}
      </h3>
      <div class="flex items-center gap-1.5">
        <Badge variant="outline" class="ml-2 text-[10px]">{{ t('jobUi.llmReadOnly') }}</Badge>
        <button
          v-if="props.closable"
          type="button"
          class="inspector-close"
          data-inspector-close
          :aria-label="t('common.close')"
          @click.stop.prevent="emit('close')"
        >
          ✕
        </button>
      </div>
    </summary>
    <div class="ins-body p-4">
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
        <details v-if="activity.agentRun?.result" class="mt-3 border-t pt-3 text-xs">
          <summary class="cursor-pointer font-medium">{{ t('jobUi.agentResult') }}</summary>
          <pre class="ins-pre mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-lg bg-muted p-2">{{ JSON.stringify(activity.agentRun.result, null, 2) }}</pre>
        </details>
      </template>
    </div>
  </details>
</template>

<style scoped>
.surface-head::-webkit-details-marker {
  display: none;
}
.inspector-close {
  width: 24px;
  height: 24px;
  border-radius: 6px;
  background: #1d2738;
  border: 1px solid #33425a;
  font-size: 11px;
  color: #a5b3ce;
  display: inline-grid;
  place-items: center;
}
.inspector-close:hover {
  background: #29364b;
}
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

<script setup lang="ts">
import { computed } from 'vue'
import { Cpu } from '@lucide/vue'
import type { JobEvent } from '@/api/types/agent-jobs'
import type { UnifiedActivity } from '@/domain/jobs/unified-activity'
import { Badge } from '@/components/ui/badge'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{ activity: UnifiedActivity | null }>()
const { t } = useI18n()
const run = computed(() => props.activity?.llm ?? null)
const step = computed(() => props.activity?.step ?? null)
const pendingStage = computed(() => {
  if (run.value?.status !== 'running') return null
  switch (run.value.events.at(-1)?.event_type) {
    case 'llm_run_started': return t('jobUi.llmAcquiringProvider')
    case 'llm_provider_acquired': return t('jobUi.llmWaitingCapacity')
    case 'llm_request_started': return t('jobUi.llmWaitingResponse')
    case 'llm_response_received': return t('jobUi.llmValidating')
    default: return null
  }
})
function duration(ms: number | null): string {
  return ms === null ? '—' : (ms / 1000).toFixed(2) + ' s'
}
function stageLabel(event: JobEvent): string {
  switch (event.event_type) {
    case 'llm_run_started': return t('jobUi.llmStarted')
    case 'llm_provider_acquired': return t('jobUi.llmProviderAcquired')
    case 'llm_request_started': return t('jobUi.llmDispatched')
    case 'llm_response_received': return t('jobUi.llmReceived')
    case 'llm_run_finished': return t('jobUi.llmFinished')
    case 'plan_rejected': return t('jobUi.llmValidationFailed')
    default: return event.event_type
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
  if (event.event_type === 'llm_run_finished') return safeDetail(event.payload.error_code ?? event.payload.validation_code)
  return ''
}
</script>

<template>
  <section class="studio-panel p-4">
    <div class="mb-3 flex items-center justify-between gap-2">
      <h3 class="flex items-center gap-2 text-sm font-semibold"><Cpu class="size-4 text-studio-violet" />{{ t('jobUi.eventInspector') }}</h3>
      <Badge variant="outline">{{ t('jobUi.llmReadOnly') }}</Badge>
    </div>
    <p v-if="!activity" class="text-xs text-muted-foreground">{{ t('jobUi.selectTimelineEvent') }}</p>
    <template v-else>
      <div class="mb-3 flex flex-wrap items-center justify-between gap-2">
        <strong class="break-words text-xs">{{ activity.title }}</strong>
        <span class="text-[10px] font-semibold capitalize" :class="activity.status === 'completed' ? 'text-success' : activity.status === 'failed' || activity.status === 'unknown' ? 'text-danger' : 'text-studio-cyan'">{{ activity.status }}</span>
      </div>
      <template v-if="run">
        <dl class="grid grid-cols-2 gap-3 text-xs">
          <div><dt class="text-muted-foreground">{{ t('jobUi.llmProvider') }}</dt><dd class="mt-1 break-all font-semibold">{{ run.provider }}</dd></div>
          <div><dt class="text-muted-foreground">{{ t('jobUi.llmModel') }}</dt><dd class="mt-1 break-all font-semibold">{{ run.model ?? '—' }}</dd></div>
          <div><dt class="text-muted-foreground">{{ t('jobUi.llmAdapter') }}</dt><dd class="mt-1 font-semibold">{{ run.adapter ?? '—' }}</dd></div>
          <div><dt class="text-muted-foreground">{{ t('jobUi.llmRole') }}</dt><dd class="mt-1 font-semibold">{{ run.role }}</dd></div>
          <div><dt class="text-muted-foreground">{{ t('jobUi.llmTransport') }}</dt><dd class="mt-1 font-semibold">{{ run.transport ?? '—' }}</dd></div>
          <div><dt class="text-muted-foreground">{{ t('jobUi.llmSource') }}</dt><dd class="mt-1 font-semibold">{{ run.providerSource }}</dd></div>
          <div><dt class="text-muted-foreground">{{ t('jobUi.llmDuration') }}</dt><dd class="mt-1 font-semibold">{{ duration(run.totalDurationMs ?? run.requestDurationMs) }}</dd></div>
          <div><dt class="text-muted-foreground">{{ t('jobUi.llmTokens') }}</dt><dd class="mt-1 font-semibold">{{ run.inputTokens ?? '—' }} / {{ run.outputTokens ?? '—' }}</dd></div>
          <div class="col-span-2"><dt class="text-muted-foreground">{{ t('jobUi.llmRunId') }}</dt><dd class="mt-1 break-all font-mono text-[11px]">{{ run.id }}</dd></div>
          <div v-if="run.events[0]?.payload.turn_id" class="col-span-2"><dt class="text-muted-foreground">{{ t('jobUi.agentTurnId') }}</dt><dd class="mt-1 break-all font-mono text-[11px]">{{ run.events[0]?.payload.turn_id }}</dd></div>
          <div v-if="activity.agentRunId" class="col-span-2"><dt class="text-muted-foreground">{{ t('jobUi.agentRunId') }}</dt><dd class="mt-1 break-all font-mono text-[11px]">{{ activity.agentRunId }}</dd></div>
        </dl>
        <div class="mt-4 border-t pt-3">
          <h4 class="mb-2 text-[10px] font-bold uppercase tracking-wide text-muted-foreground">{{ t('jobUi.llmTimeline') }}</h4>
          <div v-for="stage in run.events" :key="stage.sequence" class="flex items-start justify-between gap-2 border-l-2 border-primary/30 py-1 pl-2 text-[11px]">
            <span class="min-w-0 break-words">{{ stageLabel(stage) }}<span v-if="stage.payload.validation_path" class="ml-1 break-all font-mono text-danger">{{ stage.payload.validation_path }}</span></span>
            <span class="shrink-0 text-muted-foreground">{{ stageNote(stage) }}</span>
          </div>
          <p v-if="pendingStage" class="mt-2 text-xs text-studio-cyan">{{ pendingStage }}</p>
          <p v-if="run.errorCode" class="mt-2 break-all rounded-lg bg-danger/5 p-2 font-mono text-xs text-danger">{{ run.errorCode }}</p>
        </div>
      </template>
      <section v-if="step" class="mt-4 border-t pt-3">
        <h4 v-if="run" class="mb-2 text-xs font-semibold">{{ t('jobUi.llmCheckpointProgress') }}</h4>
        <dl class="grid grid-cols-2 gap-3 text-xs">
          <div v-if="step.plugin_id"><dt class="text-muted-foreground">{{ t('jobUi.plugin') }}</dt><dd class="mt-1 break-all font-semibold">{{ step.plugin_id ?? '—' }}</dd></div>
          <div v-if="step.plugin_id"><dt class="text-muted-foreground">{{ t('jobUi.operation') }}</dt><dd class="mt-1 break-all font-semibold">{{ step.operation ?? '—' }}</dd></div>
          <div><dt class="text-muted-foreground">{{ t('jobUi.status') }}</dt><dd class="mt-1 font-semibold">{{ step.status }}</dd></div>
          <div><dt class="text-muted-foreground">{{ t('jobUi.attemptState') }}</dt><dd class="mt-1 font-semibold">{{ step.attempt_state ?? '—' }}</dd></div>
          <div class="col-span-2"><dt class="text-muted-foreground">{{ t('jobUi.stepKey') }}</dt><dd class="mt-1 break-all font-mono text-[11px]">{{ step.step_key }}</dd></div>
          <div v-if="step.error_code" class="col-span-2"><dt class="text-muted-foreground">{{ t('jobUi.executionWarning') }}</dt><dd class="mt-1 break-all font-mono text-danger">{{ step.error_code }}</dd></div>
        </dl>
        <p v-if="step.attempt_state === 'unknown_outcome'" role="alert" class="mt-3 text-xs text-danger">{{ t('jobUi.unknownOutcomeWarning') }}</p>
      </section>
      <template v-if="!run && !step">
        <dl class="space-y-2 text-xs">
          <div v-if="activity.agentRunId"><dt class="text-muted-foreground">{{ t('jobUi.agentRunId') }}</dt><dd class="break-all font-mono">{{ activity.agentRunId }}</dd></div>
          <div v-if="activity.event?.payload.fork_group_id"><dt class="text-muted-foreground">{{ t('jobUi.forkGroup') }}</dt><dd class="break-all font-mono">{{ safeDetail(activity.event.payload.fork_group_id) }}</dd></div>
          <div><dt class="text-muted-foreground">{{ t('jobUi.execution') }}</dt><dd>{{ activity.note }}</dd></div>
        </dl>
      </template>
    </template>
  </section>
</template>

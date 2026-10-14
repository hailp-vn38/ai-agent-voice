import type { AdminJob, JobEvent } from '@/api/types/agent-jobs'

export interface LlmRun {
  id: string
  number: number
  startSequence: number
  startedAt: string
  role: 'planner' | 'worker'
  phase: string
  attempt: number
  stepKey: string | null
  provider: string
  providerSource: string
  model: string | null
  adapter: string | null
  transport: string | null
  status: 'running' | 'completed' | 'failed' | 'cancelled' | 'interrupted'
  errorCode: string | null
  validationPath: string | null
  requestDurationMs: number | null
  totalDurationMs: number | null
  inputTokens: number | null
  outputTokens: number | null
  events: JobEvent[]
}
const text = (value: unknown): string | null => typeof value === 'string' && value.length > 0 ? value : null
const int = (value: unknown): number | null =>
  typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : null

export function projectLlmRuns(events: JobEvent[], status: AdminJob['status']): LlmRun[] {
  const runs = new Map<string, LlmRun>()
  const terminal = ['succeeded', 'partial_success', 'failed', 'cancelled'].includes(status)
  for (const event of [...events].sort((a, b) => a.sequence - b.sequence)) {
    const id = text(event.payload?.run_id)
    if (!id) continue
    if (event.event_type === 'llm_run_started') {
      if (runs.has(id)) continue
      runs.set(id, {
        id, number: runs.size + 1, startSequence: event.sequence, startedAt: event.created_at,
        role: event.payload.role === 'worker' ? 'worker' : 'planner',
        phase: text(event.payload.phase) ?? 'initial',
        attempt: int(event.payload.attempt_no) ?? 1,
        stepKey: event.step_key, provider: text(event.payload.provider_ref) ?? '—',
        providerSource: text(event.payload.provider_source) ?? '—',
        model: null, adapter: null, transport: null, status: 'running',
        errorCode: null, validationPath: null, requestDurationMs: null, totalDurationMs: null,
        inputTokens: null, outputTokens: null, events: [event],
      })
      continue
    }
    const run = runs.get(id)
    if (!run) continue
    if (event.event_type.startsWith('llm_') || event.event_type === 'plan_rejected') run.events.push(event)
    if (event.event_type === 'llm_provider_acquired') {
      run.adapter = text(event.payload.adapter)
      run.model = text(event.payload.model)
      run.transport = text(event.payload.transport)
    } else if (event.event_type === 'llm_response_received') {
      run.requestDurationMs = int(event.payload.request_duration_ms)
      run.inputTokens = int(event.payload.input_tokens)
      run.outputTokens = int(event.payload.output_tokens)
    } else if (event.event_type === 'llm_run_finished') {
      const state = text(event.payload.status)
      run.status = state === 'completed' || state === 'cancelled' ? state : 'failed'
      run.totalDurationMs = int(event.payload.duration_ms)
      run.errorCode = text(event.payload.error_code) ?? text(event.payload.validation_code)
      run.validationPath = text(event.payload.validation_path)
    }
  }
  const ordered = [...runs.values()].sort((a, b) => a.startSequence - b.startSequence)
  for (const run of ordered) if (terminal && run.status === 'running') run.status = 'interrupted'
  return ordered
}
export function llmTokenSummary(runs: LlmRun[]) {
  const known = runs.filter(run => run.inputTokens !== null || run.outputTokens !== null)
  return {
    tokens: known.reduce((sum, run) => sum + (run.inputTokens ?? 0) + (run.outputTokens ?? 0), 0),
    known: known.length,
    total: runs.length,
  }
}

import type { AdminJob, JobEvent, JobStep } from '@/api/types/agent-jobs'
import { projectLlmRuns, type LlmRun } from './llm-activity'

export type ActivityKind = 'llm' | 'plugin' | 'fork' | 'join' | 'agent' | 'job' | 'plan'
export type ActivityStatus = 'running' | 'completed' | 'failed' | 'interrupted' | 'cancelled' | 'skipped' | 'unknown'
export type ActivityFilter = { kind: 'all' } | { kind: 'role'; role: string } | { kind: 'agent'; id: string }

export interface UnifiedActivity {
  id: string
  sequence: number
  time: string
  kind: ActivityKind
  status: ActivityStatus
  agentRunId: string | null
  role: string | null
  title: string
  note: string
  llm?: LlmRun
  step?: JobStep
  event?: JobEvent
}

const text = (value: unknown): string | null =>
  typeof value === 'string' && value.length > 0 ? value : null

function stepActivityStatus(step: JobStep | undefined, event: JobEvent): ActivityStatus {
  if (step?.attempt_state === 'unknown_outcome') return 'unknown'
  // The read view owns Step state; event payload is a fallback for absent Steps.
  const status = step?.status ?? text(event.payload.status)
  if (status === 'succeeded') return 'completed'
  if (status === 'failed' || status === 'cancelled' || status === 'skipped') return status
  return event.event_type === 'step_finished' && !status ? 'unknown' : 'running'
}

export function projectUnifiedActivity(job: AdminJob, events: JobEvent[]): UnifiedActivity[] {
  const ordered = [...events].sort((a, b) => a.sequence - b.sequence)
  const runs = projectLlmRuns(ordered, job.status)
  const steps = new Map(job.steps.map(step => [step.step_key, step]))
  const roles = new Map((job.agent_runs ?? []).map(run => [run.agent_run_id, run.role]))
  const items: UnifiedActivity[] = runs.map(run => {
    const first = run.events[0]
    const agentRunId = text(first?.payload.agent_run_id)
    return {
      id: 'llm:' + run.id, sequence: run.startSequence, time: run.startedAt,
      kind: 'llm', status: run.status, agentRunId,
      role: agentRunId ? roles.get(agentRunId) ?? null : null,
      title: 'LLM Run ' + String(run.number).padStart(2, '0'),
      note: run.phase, llm: run,
      step: run.stepKey ? steps.get(run.stepKey) : undefined,
    }
  })
  const llmStepKeys = new Set(runs.map(run => run.stepKey).filter((key): key is string => key !== null))
  const pluginRows = new Map<string, UnifiedActivity>()
  for (const event of ordered) {
    const payload = event.payload ?? {}
    const agentRunId = text(payload.agent_run_id) ?? text(payload.root_agent_run_id) ??
      (event.event_type === 'job_finished' ? (job.agent_runs ?? []).find(run => run.role === 'orchestrator')?.agent_run_id ?? null : null)
    const role = agentRunId ? roles.get(agentRunId) ?? null : null
    if (event.event_type === 'step_started' || event.event_type === 'step_finished') {
      if (!event.step_key) continue
      const step = steps.get(event.step_key)
      // Render an old Worker Step only when no traced LLM invocation represents it.
      if (step && !step.plugin_id && llmStepKeys.has(event.step_key)) continue
      const id = 'step:' + event.step_key
      const previous = pluginRows.get(id)
      if (previous) {
        if (event.event_type === 'step_finished') {
          previous.status = stepActivityStatus(step, event)
          previous.note = text(payload.error_code) ?? previous.note
        }
        continue
      }
      const item: UnifiedActivity = {
        id, sequence: event.sequence, time: event.created_at, kind: step && !step.plugin_id ? 'llm' : 'plugin',
        status: stepActivityStatus(step, event),
        agentRunId: agentRunId ?? step?.agent_run_id ?? null,
        role: role ?? (step?.agent_run_id ? roles.get(step.agent_run_id) ?? null : null),
        title: step?.plugin_id ? step.plugin_id + '.' + (step.operation ?? 'unknown') : step ? 'Worker Step · ' + event.step_key : event.step_key,
        note: text(payload.error_code) ?? (step && !step.plugin_id ? 'Legacy worker execution' : 'Plugin operation'),
        step, event,
      }
      pluginRows.set(id, item)
      items.push(item)
      continue
    }
    const make = (kind: ActivityKind, id: string, title: string, note: string, status: ActivityStatus = 'completed') =>
      items.push({ id, sequence: event.sequence, time: event.created_at, kind, status, agentRunId, role, title, note, event })
    if (event.event_type === 'agent_decision_accepted' && payload.action === 'delegate') {
      make('fork', 'fork:' + (text(payload.fork_group_id) ?? event.sequence), 'Delegate / Fork', 'Sub-agents: ' + (payload.task_count ?? '—'))
    } else if (event.event_type === 'join_completed') {
      make('join', 'join:' + (text(payload.fork_group_id) ?? event.sequence), 'Join completed', 'Children: ' + (payload.member_count ?? '—'))
    } else if (event.event_type === 'agent_run_finished') {
      make('agent', 'agent:' + (agentRunId ?? event.sequence), 'Agent finished', text(payload.error_code) ?? 'Run checkpoint',
        payload.status === 'succeeded' || payload.status === 'completed' ? 'completed' : payload.status === 'cancelled' ? 'cancelled' : 'failed')
    } else if (event.event_type === 'job_finished') {
      make('job', 'job:terminal', 'Job finished', text(payload.error_code) ?? text(payload.status) ?? 'Terminal checkpoint',
        payload.status === 'succeeded' || payload.status === 'partial_success' ? 'completed' : payload.status === 'cancelled' ? 'cancelled' : 'failed')
    } else if (event.event_type === 'plan_accepted' || event.event_type === 'plan_rejected') {
      make('plan', 'plan:' + event.sequence, event.event_type === 'plan_accepted' ? 'Plan accepted' : 'Plan rejected',
        text(payload.code) ?? 'Legacy DAG', event.event_type === 'plan_rejected' ? 'failed' : 'completed')
    }
  }
  return items.sort((a, b) => a.sequence - b.sequence)
}

export function visibleActivities(items: UnifiedActivity[], filter: ActivityFilter): UnifiedActivity[] {
  if (filter.kind === 'all') return items
  if (filter.kind === 'agent') return items.filter(item => item.agentRunId === filter.id)
  return items.filter(item => item.role === filter.role)
}

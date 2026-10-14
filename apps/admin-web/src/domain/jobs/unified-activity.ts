import type { AdminJob, JobEvent, JobStep } from '@/api/types/agent-jobs'
import { projectLlmRuns, type LlmRun } from './llm-activity'

export type ActivityKind = 'llm' | 'plugin' | 'fork' | 'join' | 'agent' | 'job' | 'plan'
export type ActivityStatus = 'running' | 'completed' | 'failed' | 'interrupted' | 'cancelled' | 'unknown'
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

export function projectUnifiedActivity(job: AdminJob, events: JobEvent[]): UnifiedActivity[] {
  const ordered = [...events].sort((a, b) => a.sequence - b.sequence)
  const runs = projectLlmRuns(ordered, job.status)
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
    }
  })
  const steps = new Map(job.steps.map(step => [step.step_key, step]))
  const pluginRows = new Map<string, UnifiedActivity>()
  for (const event of ordered) {
    const payload = event.payload ?? {}
    const agentRunId = text(payload.agent_run_id) ?? text(payload.root_agent_run_id) ??
      (event.event_type === 'job_finished' ? (job.agent_runs ?? []).find(run => run.role === 'orchestrator')?.agent_run_id ?? null : null)
    const role = agentRunId ? roles.get(agentRunId) ?? null : null
    if (event.event_type === 'step_started' || event.event_type === 'step_finished') {
      if (!event.step_key) continue
      const step = steps.get(event.step_key)
      // A legacy LLM Worker Step is already represented by its LLM invocation.
      if (step && !step.plugin_id) continue
      const id = 'step:' + event.step_key
      const previous = pluginRows.get(id)
      if (previous) {
        if (event.event_type === 'step_finished') {
          previous.status = step?.status === 'succeeded' || payload.status === 'succeeded' ? 'completed' : 'failed'
          if (step?.attempt_state === 'unknown_outcome') previous.status = 'unknown'
          previous.note = text(payload.error_code) ?? previous.note
        }
        continue
      }
      const status: ActivityStatus = step?.status === 'succeeded' ? 'completed'
        : step?.status === 'failed' || step?.status === 'skipped' || step?.status === 'cancelled' ? 'failed'
        : event.event_type === 'step_finished' ? payload.status === 'succeeded' ? 'completed' : 'failed' : 'running'
      const item: UnifiedActivity = {
        id, sequence: event.sequence, time: event.created_at, kind: 'plugin',
        status: step?.attempt_state === 'unknown_outcome' ? 'unknown' : status,
        agentRunId: agentRunId ?? step?.agent_run_id ?? null,
        role: role ?? (step?.agent_run_id ? roles.get(step.agent_run_id) ?? null : null),
        title: step?.plugin_id ? step.plugin_id + '.' + (step.operation ?? 'unknown') : event.step_key,
        note: text(payload.error_code) ?? 'Plugin operation',
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

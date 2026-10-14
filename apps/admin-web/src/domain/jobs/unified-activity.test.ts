import { describe, expect, it } from 'vitest'
import type { AdminJob, JobEvent } from '@/api/types/agent-jobs'
import { projectUnifiedActivity, visibleActivities } from './unified-activity'

const job = { id: 'job', status: 'running', steps: [
  { step_key: 'fetch-a', plugin_id: 'web-extractor', operation: 'fetch_url', status: 'succeeded', attempt_state: 'finished' },
  { step_key: 'fetch-b', plugin_id: 'web-extractor', operation: 'fetch_url', status: 'succeeded', attempt_state: 'finished' },
], agent_runs: [
  { agent_run_id: 'root', role: 'orchestrator' },
  { agent_run_id: 'child', role: 'research' },
] } as AdminJob
const event = (sequence: number, event_type: JobEvent['event_type'], payload: Record<string, unknown>, step_key: string | null = null): JobEvent => ({
  sequence, event_type, payload, step_key, created_at: '2026-10-14T09:42:00Z',
})
describe('unified Job activity', () => {
  it('groups LLM lifecycle and plugin start/finish into single rows with stable sequence', () => {
    const events = [
      event(5, 'step_finished', { status: 'succeeded', agent_run_id: 'child' }, 'fetch-a'),
      event(3, 'llm_run_finished', { run_id: 'llm1', status: 'completed' }),
      event(1, 'llm_run_started', { run_id: 'llm1', role: 'worker', agent_run_id: 'child' }),
      event(4, 'step_started', { agent_run_id: 'child' }, 'fetch-a'),
      event(6, 'step_started', { agent_run_id: 'child' }, 'fetch-b'),
      event(7, 'step_finished', { status: 'succeeded', agent_run_id: 'child' }, 'fetch-b'),
    ]
    const rows = projectUnifiedActivity(job, events)
    expect(rows.map(row => row.id)).toEqual(['llm:llm1', 'step:fetch-a', 'step:fetch-b'])
    expect(rows.map(row => row.status)).toEqual(['completed', 'completed', 'completed'])
    expect(visibleActivities(rows, { kind: 'role', role: 'research' })).toHaveLength(3)
    expect(visibleActivities(rows, { kind: 'agent', id: 'root' })).toEqual([])
  })
  it('preserves legacy events without inventing an Agent Run', () => {
    const legacy = { ...job, agent_runs: undefined, steps: [] } as AdminJob
    const rows = projectUnifiedActivity(legacy, [
      event(1, 'plan_accepted', { revision: 1 }),
      event(2, 'llm_run_started', { run_id: 'old', role: 'planner' }),
    ])
    expect(rows).toHaveLength(2)
    expect(rows.every(row => row.agentRunId === null)).toBe(true)
  })
})

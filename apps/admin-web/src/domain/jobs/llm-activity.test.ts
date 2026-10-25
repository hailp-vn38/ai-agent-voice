import { describe, expect, it } from 'vitest'
import type { JobEvent } from '@/api/types/agent-jobs'
import { llmTokenSummary, projectLlmRuns } from './llm-activity'

const event = (sequence: number, type: JobEvent['event_type'], runId: string, other: Record<string, unknown> = {}): JobEvent => ({
  sequence, event_type: type, created_at: '2026-10-14T02:42:01Z', step_key: null,
  payload: { run_id: runId, ...other },
})

describe('projectLlmRuns', () => {
  it('groups provider identity and optional usage by run_id, ordered by sequence', () => {
    const events = [
      event(5, 'llm_run_finished', 'a', { status: 'failed', validation_code: 'invalid_plan' }),
      event(2, 'llm_run_started', 'b', { role: 'worker', provider_ref: 'worker', phase: 'step' }),
      event(4, 'llm_response_received', 'a', { input_tokens: 100, output_tokens: 30 }),
      event(1, 'llm_run_started', 'a', { role: 'planner', provider_ref: 'config:llm:primary', phase: 'initial' }),
      event(3, 'llm_provider_acquired', 'a', { adapter: 'openai', model: 'gpt-test', transport: 'responses' }),
    ]
    const runs = projectLlmRuns(events, 'running')
    expect(runs).toHaveLength(2)
    expect(runs[0]).toMatchObject({
      id: 'a', number: 1, provider: 'config:llm:primary', model: 'gpt-test',
      transport: 'responses', status: 'failed', errorCode: 'invalid_plan',
    })
    expect(runs[1]).toMatchObject({ id: 'b', number: 2, status: 'running', provider: 'worker' })
    expect(llmTokenSummary(runs)).toEqual({ tokens: 130, known: 1, total: 2 })
  })

  it('shows field-level V2 decision rejection after a completed LLM HTTP response', () => {
    const events = [
      event(1, 'llm_run_started', 'root-1', { role: 'planner', agent_run_id: 'root', phase: 'agent_decision' }),
      event(2, 'llm_run_finished', 'root-1', { status: 'completed' }),
      event(3, 'agent_decision_rejected', 'root-1', {
        agent_run_id: 'root', validation_code: 'missing_field', validation_path: '$.tasks[0].task_key', attempt_no: 1,
      }),
    ]
    const runs = projectLlmRuns(events, 'failed')
    expect(runs).toHaveLength(1)
    expect(runs[0]).toMatchObject({ status: 'failed', errorCode: 'missing_field', validationPath: '$.tasks[0].task_key' })
    expect(runs[0]?.events.map(e => e.event_type)).toEqual(['llm_run_started','llm_run_finished','agent_decision_rejected'])
  })

  it('marks a started but unconfirmed run interrupted only for a terminal job', () => {
    const start = event(1, 'llm_run_started', 'x', { role: 'planner', provider_ref: 'p' })
    expect(projectLlmRuns([start], 'planning')[0]?.status).toBe('running')
    expect(projectLlmRuns([start], 'failed')[0]?.status).toBe('interrupted')
  })

  it('preserves old jobs and ignores orphan/malformed events', () => {
    const invalid = event(2, 'llm_provider_acquired', 'missing', { adapter: 'openai' })
    const old = event(1, 'plan_attempt', 'old')
    expect(projectLlmRuns([old, invalid], 'succeeded')).toEqual([])
    expect(llmTokenSummary([])).toEqual({ tokens: 0, known: 0, total: 0 })
  })

  it('projects orchestrator_agent runs from agent_run_started and agent_run_finished with token usage', () => {
    const events: JobEvent[] = [
      {
        sequence: 1,
        event_type: 'agent_run_created',
        created_at: '2026-10-14T02:42:01Z',
        step_key: null,
        payload: { agent_run_id: 'root-sess', parent_agent_run_id: null, role: 'daily_root' },
      },
      {
        sequence: 2,
        event_type: 'agent_run_started',
        created_at: '2026-10-14T02:42:01Z',
        step_key: null,
        payload: { agent_run_id: 'root-sess', run_id: 'run-root', turn_id: 'turn-1', attempt: 1 },
      },
      {
        sequence: 3,
        event_type: 'agent_run_created',
        created_at: '2026-10-14T02:42:02Z',
        step_key: null,
        payload: { agent_run_id: 'child-sess', parent_agent_run_id: 'root-sess', role: 'youtube_brief' },
      },
      {
        sequence: 4,
        event_type: 'agent_run_started',
        created_at: '2026-10-14T02:42:02Z',
        step_key: null,
        payload: { agent_run_id: 'child-sess', run_id: 'run-child', turn_id: 'turn-child', attempt: 1 },
      },
      {
        sequence: 5,
        event_type: 'agent_run_finished',
        created_at: '2026-10-14T02:42:03Z',
        step_key: null,
        payload: {
          agent_run_id: 'child-sess',
          run_id: 'run-child',
          status: 'succeeded',
          input_tokens: 120,
          output_tokens: 80,
        },
      },
    ]
    const runs = projectLlmRuns(events, 'running')
    expect(runs).toHaveLength(2)
    expect(runs[0]).toMatchObject({
      id: 'run-root',
      number: 1,
      role: 'planner',
      phase: 'daily_root',
      status: 'running',
    })
    expect(runs[1]).toMatchObject({
      id: 'run-child',
      number: 2,
      role: 'worker',
      phase: 'youtube_brief',
      status: 'completed',
      inputTokens: 120,
      outputTokens: 80,
    })
    expect(llmTokenSummary(runs)).toEqual({ tokens: 200, known: 1, total: 2 })
  })
})

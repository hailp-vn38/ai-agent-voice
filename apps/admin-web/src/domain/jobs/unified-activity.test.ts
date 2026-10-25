import { describe, expect, it } from 'vitest'
import type { AdminJob, JobEvent } from '@/api/types/agent-jobs'
import { groupTimelineActivities, layoutDagCanvas, projectUnifiedActivity, visibleActivities } from './unified-activity'

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
  it('displays a rejected Root decision in its existing LLM timeline row', () => {
    const activities = projectUnifiedActivity({ ...job, status: 'failed' }, [
      event(1, 'llm_run_started', { run_id: 'r-1', role: 'planner', agent_run_id: 'root' }),
      event(2, 'llm_run_finished', { run_id: 'r-1', status: 'completed' }),
      event(3, 'agent_decision_rejected', {
        run_id: 'r-1', agent_run_id: 'root', validation_code: 'invalid_role', validation_path: '$.tasks[0].role',
      }),
    ])
    expect(activities).toHaveLength(1)
    expect(activities[0]).toMatchObject({
      id: 'llm:r-1', role: 'orchestrator', status: 'failed', note: 'invalid_role · $.tasks[0].role',
    })
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
  it('attributes Join and Job finish to the actual root while children interleave', () => {
    const events = [
      event(1, 'llm_run_started', { run_id: 'root-llm', role: 'planner', agent_run_id: 'root' }),
      event(2, 'agent_decision_accepted', { action: 'delegate', agent_run_id: 'root', fork_group_id: 'fork-1', task_count: 2 }),
      event(3, 'llm_run_started', { run_id: 'child-llm', role: 'worker', agent_run_id: 'child' }),
      event(4, 'llm_run_finished', { run_id: 'child-llm', status: 'completed' }),
      event(5, 'join_completed', { root_agent_run_id: 'root', fork_group_id: 'fork-1', member_count: 2 }),
      event(6, 'job_finished', { status: 'succeeded' }),
    ]
    const rows = projectUnifiedActivity({ ...job, status: 'succeeded' }, events)
    expect(rows.map(row => row.id)).toEqual([
      'llm:root-llm', 'fork:fork-1', 'llm:child-llm', 'join:fork-1', 'job:terminal',
    ])
    expect(visibleActivities(rows, { kind: 'role', role: 'orchestrator' }).map(row => row.id))
      .toEqual(['llm:root-llm', 'fork:fork-1', 'join:fork-1', 'job:terminal'])
    expect(visibleActivities(rows, { kind: 'agent', id: 'child' }).map(row => row.id))
      .toEqual(['llm:child-llm'])
  })
  it('reports an ambiguous tool outcome as unknown, never completed', () => {
    const ambiguous = {
      ...job, status: 'failed', steps: [{
        ...job.steps[0]!, status: 'failed', attempt_state: 'unknown_outcome',
      }],
    } as AdminJob
    const rows = projectUnifiedActivity(ambiguous, [
      event(1, 'step_started', { agent_run_id: 'child' }, 'fetch-a'),
      event(2, 'step_finished', { status: 'failed', error_code: 'worker_unreaped' }, 'fetch-a'),
    ])
    expect(rows).toHaveLength(1)
    expect(rows[0]).toMatchObject({ id: 'step:fetch-a', status: 'unknown', role: 'research' })
  })

  it('shows legacy worker Steps when old six-event jobs have no LLM trace', () => {
    const legacy = { ...job, agent_runs: undefined, steps: [{
      ...job.steps[0]!, plugin_id: null, operation: null, status: 'succeeded',
    }] } as AdminJob
    const rows = projectUnifiedActivity(legacy, [
      event(1, 'step_started', { attempt_state: 'reserved' }, 'fetch-a'),
      event(2, 'step_finished', { status: 'succeeded' }, 'fetch-a'),
      event(3, 'job_finished', { status: 'succeeded' }),
    ])
    expect(rows.map(row => row.id)).toEqual(['step:fetch-a', 'job:terminal'])
    expect(rows[0]).toMatchObject({ kind: 'llm', status: 'completed', agentRunId: null })
    const traced = projectUnifiedActivity(legacy, [
      event(1, 'llm_run_started', { run_id: 'worker', role: 'worker' }, 'fetch-a'),
      event(2, 'step_started', {}, 'fetch-a'),
      event(3, 'step_finished', { status: 'succeeded' }, 'fetch-a'),
    ])
    expect(traced.map(row => row.id)).toEqual(['llm:worker'])
  })

  it.each(['running', 'failed', 'cancelled'] as const)('keeps %s Worker checkpoint separate from a completed invocation on partial pages', status => {
    const step = { ...job.steps[0]!, plugin_id: null, status,
      attempt_state: status === 'failed' ? 'unknown_outcome' : null }
    const worker = { ...job, steps: [step] } as AdminJob
    const trace = [
      event(1, 'llm_run_started', { run_id: 'worker', role: 'worker' }, 'fetch-a'),
      event(2, 'llm_run_finished', { run_id: 'worker', status: 'completed' }, 'fetch-a'),
    ]
    for (const events of [trace, [...trace, event(3, 'step_finished', { status }, 'fetch-a')]]) {
      const rows = projectUnifiedActivity(worker, events)
      expect(rows).toHaveLength(1)
      expect(rows[0]).toMatchObject({ status: 'completed', llm: { status: 'completed' }, step })
    }
  })

  it.each(['cancelled', 'skipped'] as const)('preserves %s Steps with and without a read-view entry', status => {
    const step = { ...job.steps[0]!, status }
    for (const steps of [[step], []]) {
      const view = { ...job, steps } as AdminJob
      const started = event(1, 'step_started', {}, 'fetch-a')
      const finished = event(2, 'step_finished', { status }, 'fetch-a')
      for (const events of [[finished], [started, finished]]) {
        expect(projectUnifiedActivity(view, events)[0]?.status).toBe(status)
      }
    }
  })

  it('uses authoritative cancellation even when an earlier finish event reports success', () => {
    const view = { ...job, steps: [{ ...job.steps[0]!, status: 'cancelled' }] } as AdminJob
    expect(projectUnifiedActivity(view, [
      event(1, 'step_started', {}, 'fetch-a'),
      event(2, 'step_finished', { status: 'succeeded' }, 'fetch-a'),
    ])[0]?.status).toBe('cancelled')
  })

  it('correlates native turn IDs, deduplicates child creation and shares Join counts', () => {
    const native = [
      event(1, 'agent_decision_accepted', { agent_run_id: 'root', action: 'delegate', turn_id: 'turn' }),
      event(2, 'agent_run_created', { agent_run_id: 'child', parent_agent_run_id: 'root', role: 'research', fork_group_id: 'turn' }),
      event(3, 'agent_run_finished', { agent_run_id: 'child', status: 'succeeded' }),
      event(4, 'join_completed', { root_agent_run_id: 'root', fork_group_id: 'turn', member_count: 1 }),
    ]
    for (const events of [native.slice(0, 3), native, [...native, native[1]!]]) {
      const rows = projectUnifiedActivity(job, events)
      expect(rows[0]).toMatchObject({ id: 'fork:turn', memberCount: 1, note: 'Sub-agents: 1' })
      expect(groupTimelineActivities(rows)[1]).toMatchObject({ id: 'child', child: true, activities: [{ title: 'Agent finished' }] })
      if (events.length >= 4) expect(rows.at(-1)).toMatchObject({ memberCount: 1, note: 'Children: 1' })
    }
  })

  it('uses available Join membership for both checkpoints without inventing children', () => {
    const rows = projectUnifiedActivity(job, [
      event(1, 'agent_decision_accepted', { agent_run_id: 'root', action: 'delegate', fork_group_id: 'fork', task_count: 5 }),
      event(2, 'join_completed', { root_agent_run_id: 'root', fork_group_id: 'fork', member_count: 1 }),
    ])
    expect(rows.map(row => row.memberCount)).toEqual([1, 1])
    expect(rows.every(row => row.childRuns?.length === 0)).toBe(true)
    expect(projectUnifiedActivity(job, [event(1, 'agent_decision_accepted', { action: 'delegate' })])[0]?.memberCount).toBeNull()
    expect(projectUnifiedActivity(job, [event(1, 'agent_decision_accepted', { action: 'delegate', task_count: 2 })])[0]?.memberCount).toBe(2)
  })

  it('lays out Root → Fork → Parallel Child Swimlanes → Join on a 2D DAG canvas and filters by kind', () => {
    const dagJob = {
      ...job,
      agent_runs: [
        { agent_run_id: 'root', role: 'orchestrator', parent_agent_run_id: null },
        { agent_run_id: 'child-a', role: 'research', parent_agent_run_id: 'root', fork_group_id: 'fork-1' },
        { agent_run_id: 'child-b', role: 'document', parent_agent_run_id: 'root', fork_group_id: 'fork-1' },
      ],
    } as AdminJob
    const rows = projectUnifiedActivity(dagJob, [
      event(1, 'llm_run_started', { run_id: 'plan', role: 'planner', agent_run_id: 'root' }),
      event(2, 'agent_decision_accepted', { run_id: 'plan', agent_run_id: 'root', action: 'delegate', fork_group_id: 'fork-1', task_count: 2 }),
      event(3, 'llm_run_started', { run_id: 'res-llm', role: 'worker', agent_run_id: 'child-a' }),
      event(4, 'llm_run_finished', { run_id: 'res-llm', status: 'completed' }),
      event(5, 'step_started', { agent_run_id: 'child-a' }, 'fetch-a'),
      event(6, 'step_finished', { status: 'succeeded', agent_run_id: 'child-a' }, 'fetch-a'),
      event(7, 'llm_run_started', { run_id: 'doc-llm', role: 'worker', agent_run_id: 'child-b' }),
      event(8, 'join_completed', { root_agent_run_id: 'root', fork_group_id: 'fork-1', member_count: 2 }),
      event(9, 'job_finished', { status: 'running' }),
    ])
    expect(visibleActivities(rows, { kind: 'llm' }).map(r => r.id)).toEqual(['llm:plan', 'llm:res-llm', 'llm:doc-llm'])
    expect(visibleActivities(rows, { kind: 'plugin' }).map(r => r.id)).toEqual(['step:fetch-a'])
    expect(visibleActivities(rows, { kind: 'root' }).map(r => r.id)).toEqual(['llm:plan', 'fork:fork-1', 'join:fork-1', 'job:terminal'])

    const dag = layoutDagCanvas(rows)
    expect(dag.lanes).toHaveLength(2)
    expect(dag.lanes[0]).toMatchObject({ id: 'child-a', theme: 'research', y: 48 })
    expect(dag.lanes[1]).toMatchObject({ id: 'child-b', theme: 'document', y: 454 })
    expect(dag.lanes[0]!.nodes[0]!.x).toBe(dag.lanes[0]!.nodes[1]!.x)
    expect(dag.lanes[0]!.nodes[1]!.y).toBeGreaterThan(
      dag.lanes[0]!.nodes[0]!.y + dag.lanes[0]!.nodes[0]!.height,
    )
    for (const lane of dag.lanes) {
      for (const node of lane.nodes) {
        expect(node.x).toBeGreaterThanOrEqual(lane.x + 20)
        expect(node.x + node.width).toBeLessThanOrEqual(lane.x + lane.width - 20)
        expect(node.y).toBeGreaterThanOrEqual(lane.y + 58)
        expect(node.y + node.height).toBeLessThanOrEqual(lane.y + lane.height - 20)
      }
    }
    expect(dag.edges.map(e => `${e.fromId}->${e.toId}`)).toEqual([
      'llm:res-llm->step:fetch-a',
      'llm:plan->fork:fork-1',
      'fork:fork-1->llm:res-llm',
      'fork:fork-1->llm:doc-llm',
      'step:fetch-a->join:fork-1',
      'llm:doc-llm->join:fork-1',
      'join:fork-1->job:terminal',
    ])

    const collapsedDag = layoutDagCanvas(rows, new Set(['child-a', 'child-b']))
    const topMargin = collapsedDag.lanes[0]!.y
    const bottomMargin =
      collapsedDag.height - (collapsedDag.lanes[1]!.y + collapsedDag.lanes[1]!.height)
    expect(topMargin).toBe(bottomMargin)
    const rootNodes = collapsedDag.nodes.filter(node => node.laneId === null)
    for (const rootNode of rootNodes) {
      expect(Math.round(rootNode.y + rootNode.height / 2)).toBe(Math.round(collapsedDag.height / 2))
    }
  })

})

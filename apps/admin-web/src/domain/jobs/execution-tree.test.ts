import { describe, expect, it } from 'vitest'
import type { AdminJob, JobEvent } from '@/api/types/agent-jobs'
import {
  buildExecutionTree,
  filterVisibleTreeNodes,
  formatDurationMs,
  formatSpanDuration,
  treeNodeAncestors,
  treeNodeChildren,
  treeNodeDescendants,
} from './execution-tree'
import { projectUnifiedActivity } from './unified-activity'

describe('execution-tree domain projection', () => {
  it('builds a depth-first pre-order hierarchy for Root -> Subagent L1 -> Subagent L2 -> Tool & LLM calls', () => {
    const job: AdminJob = {
      id: 'job-tree-1',
      title: 'Knowledge · Source Ingest',
      goal: 'Verify knowledge claims and write wiki drafts',
      status: 'running',
      created_at: '1700000000',
      started_at: '1700000000',
      finished_at: null,
      requester_scope: 'admin',
      plan_revision: 1,
      retry_of_job_id: null,
      result: null,
      budget_used: { total_tokens: 4200 },
      artifacts: [],
      agent_runs: [
        {
          agent_run_id: 'root',
          parent_agent_run_id: null,
          role: 'orchestrator',
          goal_excerpt: 'Coordinate ingest, research, and wiki writer',
          status: 'running',
          run_index: 0,
          llm_rounds: 1,
          tool_calls: 0,
          input_tokens: 500,
          output_tokens: 150,
          started_at: '1700000000',
          finished_at: null,
        },
        {
          agent_run_id: 'ingest',
          parent_agent_run_id: 'root',
          role: 'wiki_ingestor',
          goal_excerpt: 'Normalize source files',
          status: 'succeeded',
          run_index: 1,
          llm_rounds: 1,
          tool_calls: 1,
          input_tokens: 800,
          output_tokens: 200,
          started_at: '1700000004',
          finished_at: '1700000072',
        },
        {
          agent_run_id: 'research',
          parent_agent_run_id: 'root',
          role: 'research',
          goal_excerpt: 'Cross-check claims against sources',
          status: 'running',
          run_index: 2,
          llm_rounds: 1,
          tool_calls: 0,
          input_tokens: 900,
          output_tokens: 300,
          started_at: '1700000073',
          finished_at: null,
        },
        {
          agent_run_id: 'audit',
          parent_agent_run_id: 'research',
          role: 'citation_audit',
          goal_excerpt: 'Audit conflicting citations',
          status: 'running',
          run_index: 3,
          llm_rounds: 1,
          tool_calls: 1,
          input_tokens: 400,
          output_tokens: 100,
          started_at: '1700000120',
          finished_at: null,
        },
      ],
      steps: [
        {
          step_key: 'read_source',
          agent_run_id: 'ingest',
          plugin_id: 'knowledge',
          operation: 'read_source',
          status: 'succeeded',
          attempt_state: 'completed',
          instruction: 'Read source src_bc7f45',
          required: true,
          dependencies: [],
          input_refs: [],
          arguments: { source_id: 'src_bc7f45' },
          output_schema: {},
          result: { files: 14, blocks: 126 },
        },
        {
          step_key: 'verify_claims',
          agent_run_id: 'audit',
          plugin_id: 'evidence',
          operation: 'verify_claims',
          status: 'running',
          attempt_state: 'running',
          instruction: 'Verify claims c-017 and c-021',
          required: true,
          dependencies: [],
          input_refs: [],
          arguments: { claim_ids: ['c-017', 'c-021'] },
          output_schema: {},
          result: null,
        },
      ],
    }

    const events: JobEvent[] = [
      {
        sequence: 1,
        event_type: 'step_finished',
        step_key: 'read_source',
        created_at: '1700000010',
        payload: { agent_run_id: 'ingest', status: 'succeeded' },
      },
      {
        sequence: 2,
        event_type: 'agent_run_finished',
        step_key: null,
        created_at: '1700000072',
        payload: { agent_run_id: 'ingest', status: 'succeeded', assistant_message: '14 files loaded.' },
      },
      {
        sequence: 3,
        event_type: 'step_started',
        step_key: 'verify_claims',
        created_at: '1700000130',
        payload: { agent_run_id: 'audit', tool_name: 'verify_claims' },
      },
    ]

    const activities = projectUnifiedActivity(job, events)
    const nodes = buildExecutionTree(activities, job)

    expect(nodes.map(n => n.id)).toEqual([
      'root',
      'agent:ingest',
      'step:read_source',
      'agent:research',
      'agent:audit',
      'step:verify_claims',
    ])

    expect(treeNodeAncestors(nodes, 'step:verify_claims').map(a => a.id)).toEqual([
      'root',
      'agent:research',
      'agent:audit',
    ])
    expect(treeNodeChildren(nodes, 'root').map(c => c.id)).toEqual(['agent:ingest', 'agent:research'])
    expect(treeNodeDescendants(nodes, 'agent:research').map(d => d.id)).toEqual([
      'agent:audit',
      'step:verify_claims',
    ])

    const ingestNode = nodes.find(n => n.id === 'agent:ingest')!
    expect(ingestNode.duration).toBe('01:08')
    expect(ingestNode.state).toBe('done')

    // Filtering by search query preserves ancestors even when collapsed
    const collapsed = new Set(['root'])
    expect(filterVisibleTreeNodes(nodes, collapsed, '').map(n => n.id)).toEqual([
      'root',
      'agent:ingest',
      'agent:research',
    ])
    expect(filterVisibleTreeNodes(nodes, collapsed, 'verify_claims').map(n => n.id)).toEqual([
      'root',
      'agent:research',
      'agent:audit',
      'step:verify_claims',
    ])
  })

  it('formats durations cleanly in seconds and MM:SS', () => {
    expect(formatDurationMs(800)).toBe('0.8s')
    expect(formatDurationMs(1600)).toBe('1.6s')
    expect(formatDurationMs(23000)).toBe('23s')
    expect(formatDurationMs(68000)).toBe('01:08')
    expect(formatSpanDuration('1700000000', '1700000272', false)).toBe('04:32')
    expect(formatSpanDuration('1700000000', null, true)).toBe('live')
  })
})

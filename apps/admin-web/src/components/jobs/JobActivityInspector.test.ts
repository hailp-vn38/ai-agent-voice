import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it } from 'vitest'
import type { AdminJob, JobEvent } from '@/api/types/agent-jobs'
import { setLocale, translate } from '@/composables/useI18n'
import { projectUnifiedActivity } from '@/domain/jobs/unified-activity'
import JobActivityInspector from './JobActivityInspector.vue'
import JobUnifiedTimeline from './JobUnifiedTimeline.vue'

const view = { id: 'job', status: 'running', steps: [] } as unknown as AdminJob
const lifecycle: JobEvent['event_type'][] = [
  'llm_run_started', 'llm_provider_acquired', 'llm_request_started', 'llm_response_received', 'llm_run_finished',
]
function events(count: number): JobEvent[] {
  return lifecycle.slice(0, count).map((event_type, index) => ({
    sequence: index + 1, event_type, step_key: 'worker', created_at: '2026-10-14T00:00:00Z',
    payload: { run_id: 'invocation', role: 'worker', status: 'completed' },
  }))
}
beforeEach(() => setLocale('en'))

describe('Job activity presentation', () => {
  it('starts collapsed, toggles from the heading and collapses for another activity', async () => {
    const activity = projectUnifiedActivity(view, events(5))[0]!
    const wrapper = mount(JobActivityInspector, { props: { activity } })
    try {
      const details = () => wrapper.get('details').element as HTMLDetailsElement
      expect(details().open).toBe(false)
      wrapper.get('summary').element.click()
      expect(details().open).toBe(true)
      expect(wrapper.text()).toContain(activity.title)
      wrapper.get('summary').element.click()
      expect(details().open).toBe(false)
      wrapper.get('summary').element.click()
      await wrapper.setProps({ activity: { ...activity, id: 'another-invocation' } })
      expect(details().open).toBe(false)
    } finally { wrapper.unmount() }
  })

  it('updates the pending message through provider, capacity, response and validation stages', async () => {
    const keys = [
      'jobUi.llmAcquiringProvider', 'jobUi.llmWaitingCapacity', 'jobUi.llmWaitingResponse', 'jobUi.llmValidating',
    ] as const
    const wrapper = mount(JobActivityInspector, { props: { activity: null } })
    try {
      for (let count = 1; count <= 5; count++) {
        await wrapper.setProps({ activity: projectUnifiedActivity(view, events(count))[0]! })
        for (const [index, key] of keys.entries()) {
          expect(wrapper.text().includes(translate(key))).toBe(index === count - 1)
        }
      }
    } finally { wrapper.unmount() }
  })

  it('shows a completed invocation alongside its failed/unknown Worker checkpoint', () => {
    const worker = { ...view, status: 'failed', steps: [{
      step_key: 'worker', plugin_id: null, status: 'failed', attempt_state: 'unknown_outcome', error_code: 'restart_interrupted',
    }] } as AdminJob
    const activities = projectUnifiedActivity(worker, events(5))
    const inspector = mount(JobActivityInspector, { props: { activity: activities[0]! } })
    const timeline = mount(JobUnifiedTimeline, { props: { activities, selectedId: activities[0]!.id, filter: { kind: 'all' }, roles: [] } })
    try {
      expect(inspector.text()).toContain('completed')
      expect(inspector.text()).toContain('failed')
      expect(inspector.text()).toContain('restart_interrupted')
      expect(inspector.get('[role="alert"]').text()).toBe(translate('jobUi.unknownOutcomeWarning'))
      expect(timeline.text()).toContain('completed')
      expect(timeline.text()).toContain('worker · failed · unknown_outcome')
    } finally { inspector.unmount(); timeline.unmount() }
  })
  it('exposes Plugin arguments and result in expandable sections', () => {
    const pluginJob = { ...view, steps: [{
      step_key: 'extract', plugin_id: 'core.web', operation: 'extract', status: 'succeeded',
      arguments: { url: 'https://example.org/article' }, result: { text: 'Article text' },
      instruction: 'Read article', required: true, dependencies: [], input_refs: [], output_schema: {},
    }] } as AdminJob
    const activity = projectUnifiedActivity(pluginJob, [{
      sequence: 1, event_type: 'step_finished', step_key: 'extract', created_at: '2026-10-15T00:00:00Z', payload: { status: 'succeeded' },
    }])[0]!
    const wrapper = mount(JobActivityInspector, { props: { activity } })
    try {
      expect(wrapper.text()).toContain('core.web')
      expect(wrapper.text()).toContain('extract')
      const sections = wrapper.findAll('details details')
      expect(sections).toHaveLength(2)
      expect(sections[0]!.get('pre').text()).toContain('https://example.org/article')
      expect(sections[1]!.get('pre').text()).toContain('Article text')
    } finally { wrapper.unmount() }
  })

  it('displays assistant tool-call message and agent run messages when present', () => {
    const jobWithMessages = {
      ...view,
      agent_runs: [{
        agent_run_id: 'sess-root-1',
        parent_agent_run_id: null,
        role: 'daily_root',
        goal_excerpt: 'Search wiki for Rust',
        status: 'running',
        run_index: 0,
        llm_rounds: 1,
        tool_calls: 1,
        input_tokens: 100,
        output_tokens: 40,
        started_at: '1760000001',
        finished_at: null,
        messages: [
          { id: 'm1', sequence: 1, role: 'user', text: 'Search wiki for Rust' },
          { id: 'm2', sequence: 2, role: 'assistant', text: 'I will search the Wiki for Rust now.', tool_call_ids: ['call-1'] },
        ],
      }],
      steps: [{
        step_key: 'daily_root:1:wiki_search',
        agent_run_id: 'sess-root-1',
        plugin_id: 'orchestrator_flow',
        operation: 'wiki_search',
        status: 'running',
        instruction: 'daily_root -> wiki_search',
        assistant_message: 'I will search the Wiki for Rust now.',
        required: true,
        dependencies: [],
        input_refs: [],
        arguments: { query: 'Rust' },
        output_schema: {},
        result: null,
      }],
    } as AdminJob
    const activity = projectUnifiedActivity(jobWithMessages, [{
      sequence: 1,
      event_type: 'step_started',
      step_key: 'daily_root:1:wiki_search',
      created_at: '2026-10-15T00:00:00Z',
      payload: {
        agent_run_id: 'sess-root-1',
        tool_name: 'wiki_search',
        assistant_message: 'I will search the Wiki for Rust now.',
      },
    }])[0]!
    const wrapper = mount(JobActivityInspector, { props: { activity, open: true } })
    try {
      expect(wrapper.text()).toContain('I will search the Wiki for Rust now.')
      expect(wrapper.text()).toContain('Search wiki for Rust')
    } finally { wrapper.unmount() }
  })

  it('renders job result and artifacts on the canvas nodes and in the inspector with preview controls', () => {
    const jobWithResultAndArtifacts = {
      ...view,
      id: 'job-art-1',
      status: 'succeeded',
      result: { text: '# Final Summary\nCompleted successfully.', summary: '# Final Summary\nCompleted successfully.' },
      artifacts: [{
        id: 'art-1',
        step_key: 'daily_root:1:markdown_create',
        filename: 'summary_report.md',
        media_type: 'text/markdown',
        size_bytes: 2048,
        sha256: 'b'.repeat(64),
        created_at: '1760000005',
      }],
      steps: [{
        step_key: 'daily_root:1:markdown_create',
        agent_run_id: 'sess-root-1',
        plugin_id: 'orchestrator_flow',
        operation: 'markdown_create',
        status: 'succeeded',
        instruction: 'daily_root -> markdown_create',
        required: true,
        dependencies: [],
        input_refs: [],
        arguments: { filename: 'summary_report.md' },
        output_schema: {},
        result: { artifact_id: 'art-1' },
      }],
    } as AdminJob
    const activities = projectUnifiedActivity(jobWithResultAndArtifacts, [
      {
        sequence: 1,
        event_type: 'step_started',
        step_key: 'daily_root:1:markdown_create',
        created_at: '2026-10-15T00:00:00Z',
        payload: { agent_run_id: 'sess-root-1', tool_name: 'markdown_create' },
      },
      {
        sequence: 2,
        event_type: 'step_finished',
        step_key: 'daily_root:1:markdown_create',
        created_at: '2026-10-15T00:00:01Z',
        payload: { agent_run_id: 'sess-root-1', status: 'succeeded' },
      },
      {
        sequence: 3,
        event_type: 'job_finished',
        step_key: null,
        created_at: '2026-10-15T00:00:02Z',
        payload: { status: 'succeeded' },
      },
    ])
    const timeline = mount(JobUnifiedTimeline, {
      props: { jobId: 'job-art-1', activities, selectedId: 'job:terminal', filter: { kind: 'all' }, roles: [] },
    })
    const inspector = mount(JobActivityInspector, {
      props: { jobId: 'job-art-1', activity: activities[1]!, open: true },
    })
    try {
      const resultCard = timeline.get('[data-step="job:result"]')
      expect(resultCard.get('[data-result-markdown]').html()).toContain('Final Summary')
      expect(resultCard.find('[data-canvas-artifact-preview]').exists()).toBe(true)
      expect(timeline.text()).toContain('📎 summary_report.md')
      expect(timeline.text()).toContain('1 artifacts')
      expect(inspector.text()).toContain('Final Summary')
      expect(inspector.text()).toContain('summary_report.md')
      expect(inspector.find('[data-artifact-preview]').exists()).toBe(true)
    } finally {
      timeline.unmount()
      inspector.unmount()
    }
  })

})

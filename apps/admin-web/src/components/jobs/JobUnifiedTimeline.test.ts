import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { agentJobsApi } from '@/api/agent-jobs'
import type { AdminJob, JobEvent } from '@/api/types/agent-jobs'
import { setLocale } from '@/composables/useI18n'
import { projectUnifiedActivity } from '@/domain/jobs/unified-activity'
import JobUnifiedTimeline from './JobUnifiedTimeline.vue'

const job = {
  id: 'job-1',
  goal: 'Verify knowledge claims',
  status: 'succeeded',
  steps: [],
  agent_runs: [
    { agent_run_id: 'root', role: 'orchestrator', parent_agent_run_id: null, goal_excerpt: 'Orchestrate research' },
    { agent_run_id: 'research-a', role: 'research', parent_agent_run_id: 'root', fork_group_id: 'turn', goal_excerpt: 'Read source A' },
    { agent_run_id: 'research-b', role: 'research', parent_agent_run_id: 'root', fork_group_id: 'turn', goal_excerpt: 'Read source B' },
  ],
} as unknown as AdminJob

const event = (sequence: number, event_type: JobEvent['event_type'], payload: Record<string, unknown>): JobEvent => ({
  sequence,
  event_type,
  payload,
  step_key: null,
  created_at: '2026-10-15T00:00:00Z',
})

const events = [
  event(1, 'agent_decision_accepted', { agent_run_id: 'root', action: 'delegate', turn_id: 'turn' }),
  event(2, 'llm_run_started', { agent_run_id: 'research-a', run_id: 'llm-a', role: 'worker' }),
  event(3, 'llm_run_started', { agent_run_id: 'research-b', run_id: 'llm-b', role: 'worker' }),
  event(4, 'agent_run_finished', { agent_run_id: 'research-b', status: 'succeeded' }),
  event(5, 'agent_run_finished', { agent_run_id: 'research-a', status: 'succeeded' }),
  event(6, 'join_completed', { root_agent_run_id: 'root', fork_group_id: 'turn', member_count: 2 }),
  event(7, 'job_finished', { status: 'succeeded' }),
]

beforeEach(() => setLocale('en'))

describe('Agent Tree Compact Job workspace', () => {
  it('renders hierarchical Execution Tree with Root -> Subagent -> LLM/Tool nodes and inline Inspector', async () => {
    const activities = projectUnifiedActivity(job, events)
    const wrapper = mount(JobUnifiedTimeline, {
      props: {
        job,
        activities,
        selectedId: null,
        filter: { kind: 'all' },
        roles: ['research'],
      },
    })
    try {
      const rows = wrapper.findAll('.tree-row')
      expect(rows.length).toBeGreaterThanOrEqual(5)
      expect(rows[0]!.attributes('data-node')).toBe('root')
      expect(rows[0]!.attributes('style')).toContain('--depth: 0')

      const branchA = wrapper.get('[data-agent-run-id="research-a"]')
      expect(branchA.text()).toContain('Sub-agent · Research')
      expect(branchA.attributes('style')).toContain('--depth: 1')
      expect(branchA.find('.sub-count').text()).toBe('1')

      const llmRowA = wrapper.get('[data-node="llm:llm-a"]')
      expect(llmRowA.attributes('style')).toContain('--depth: 2')

      // Clicking subagent row shows its goal and finish event in the inline right-hand inspector
      await branchA.trigger('click')
      const inspector = wrapper.get('[data-tree-inspector]')
      expect(inspector.text()).toContain('Read source A')
      expect(inspector.text()).toContain('Agent finished')
      expect(inspector.get('#breadcrumb').text()).toContain('Root Orchestrator')
      expect(inspector.get('#breadcrumb').text()).toContain('Sub-agent · Research')

      // Clicking join checkpoint emits select
      const joinRow = wrapper.get('[data-step="join:turn"]')
      await joinRow.trigger('click')
      expect(wrapper.emitted('select')).toContainEqual(['join:turn'])
      expect(inspector.text()).toContain('Join completed')
      expect(inspector.text()).toContain('Children: 2')

      // Role filter chip emits filter
      const roleBtn = wrapper.findAll('.filter').find(button => button.text() === 'Research')!
      await roleBtn.trigger('click')
      expect(wrapper.emitted('filter')).toEqual([[{ kind: 'role', role: 'research' }]])
    } finally {
      wrapper.unmount()
    }
  })

  it('supports multi-level nested subagents (Root -> Subagent L1 -> Subagent L2 -> Tool call) and expand/collapse controls', async () => {
    const nestedJob = {
      ...job,
      status: 'running',
      agent_runs: [
        { agent_run_id: 'root', role: 'orchestrator', parent_agent_run_id: null, goal_excerpt: 'Root goal', status: 'running' },
        { agent_run_id: 'research', role: 'research', parent_agent_run_id: 'root', goal_excerpt: 'L1 Research', status: 'running' },
        { agent_run_id: 'audit', role: 'citation_audit', parent_agent_run_id: 'research', goal_excerpt: 'L2 Citation Audit', status: 'running' },
      ],
      steps: [
        {
          step_key: 'verify',
          agent_run_id: 'audit',
          plugin_id: 'evidence',
          operation: 'verify_claims',
          status: 'running',
          instruction: 'Verify claims c-017 and c-021',
          required: true,
          dependencies: [],
          input_refs: [],
          arguments: { claim_ids: ['c-017', 'c-021'], strict: true },
          output_schema: {},
          result: null,
        },
      ],
    } as unknown as AdminJob

    const nestedEvents = [
      {
        sequence: 1,
        event_type: 'step_started' as const,
        step_key: 'verify',
        created_at: '2026-10-15T09:42:11Z',
        payload: { agent_run_id: 'audit', tool_name: 'verify_claims' },
      },
    ]

    const activities = projectUnifiedActivity(nestedJob, nestedEvents)
    const wrapper = mount(JobUnifiedTimeline, {
      props: {
        job: nestedJob,
        activities,
        selectedId: 'step:verify',
        filter: { kind: 'all' },
        roles: [],
      },
    })
    try {
      expect(wrapper.get('[data-node="root"]').attributes('style')).toContain('--depth: 0')
      expect(wrapper.get('[data-node="agent:research"]').attributes('style')).toContain('--depth: 1')
      expect(wrapper.get('[data-node="agent:audit"]').attributes('style')).toContain('--depth: 2')
      expect(wrapper.get('[data-node="step:verify"]').attributes('style')).toContain('--depth: 3')

      const inspector = wrapper.get('[data-tree-inspector]')
      expect(inspector.get('#breadcrumb').text()).toContain('Root Orchestrator')
      expect(inspector.get('#breadcrumb').text()).toContain('Sub-agent · Research')
      expect(inspector.get('#breadcrumb').text()).toContain('Sub-agent · citation_audit')
      expect(inspector.get('#breadcrumb').text()).toContain('evidence.verify_claims')
      expect(inspector.text()).toContain('c-017')

      // Collapse all keeps only root children visible
      await wrapper.get('[data-tree-collapse-all]').trigger('click')
      expect(wrapper.find('[data-node="step:verify"]').exists()).toBe(false)
      expect(wrapper.find('[data-node="agent:audit"]').exists()).toBe(false)
      expect(wrapper.find('[data-node="agent:research"]').exists()).toBe(true)

      // Expand all restores deep nodes
      await wrapper.get('[data-tree-expand-all]').trigger('click')
      expect(wrapper.find('[data-node="agent:audit"]').exists()).toBe(true)
      expect(wrapper.find('[data-node="step:verify"]').exists()).toBe(true)

      // Search filters by tool name while preserving ancestor chain
      await wrapper.get('[data-tree-collapse-all]').trigger('click')
      const searchInput = wrapper.get('[data-tree-search]')
      await searchInput.setValue('verify_claims')
      expect(wrapper.findAll('.tree-row').map(r => r.attributes('data-node'))).toEqual([
        'root',
        'agent:research',
        'agent:audit',
        'step:verify',
      ])

      await searchInput.setValue('nonexistent_xyz_node')
      expect(wrapper.find('.empty-search').exists()).toBe(true)
    } finally {
      wrapper.unmount()
    }
  })

  it('auto-follows newly running nodes when Follow ON is enabled and navigates with Prev/Next buttons', async () => {
    const runningJob = { ...job, status: 'running' as const }
    const initialActivities = projectUnifiedActivity(runningJob, events.slice(0, 2))
    const wrapper = mount(JobUnifiedTimeline, {
      props: {
        job: runningJob,
        activities: initialActivities,
        selectedId: null,
        filter: { kind: 'all' },
        roles: [],
      },
    })
    try {
      expect(wrapper.get('[data-follow-btn]').text()).toContain('Follow ON')
      expect(wrapper.get('#footerState').text()).toContain('nodes running')

      // When a new running LLM node arrives on research-b, auto-follow selects it
      const updatedActivities = projectUnifiedActivity(runningJob, events.slice(0, 3))
      await wrapper.setProps({ activities: updatedActivities })
      expect(wrapper.get('[data-tree-inspector]').attributes('data-inspected-node')).toBe('llm:llm-b')

      // Step selection with Prev / Next buttons
      await wrapper.get('[data-prev-node]').trigger('click')
      expect(wrapper.get('[data-tree-inspector]').attributes('data-inspected-node')).toBe('agent:research-b')
      await wrapper.get('[data-next-node]').trigger('click')
      expect(wrapper.get('[data-tree-inspector]').attributes('data-inspected-node')).toBe('llm:llm-b')
    } finally {
      wrapper.unmount()
    }
  })

  it('keeps legacy activity under Root when no agent_runs exist', () => {
    const activities = projectUnifiedActivity({ ...job, agent_runs: undefined }, [events[6]!])
    const wrapper = mount(JobUnifiedTimeline, {
      props: { activities, selectedId: null, filter: { kind: 'all' }, roles: [] },
    })
    try {
      expect(wrapper.find('[data-node="root"]').exists()).toBe(true)
      expect(wrapper.text()).toContain('Job finished')
    } finally {
      wrapper.unmount()
    }
  })

  it('renders Result and Media nodes in the tree and displays image/video previews in the inline Inspector', async () => {
    const artifactSpy = vi.spyOn(agentJobsApi, 'artifact').mockImplementation(async (_jobId, artId) => {
      if (artId === 'art-poster') return new Blob(['png-bytes'], { type: 'image/png' })
      return new Blob(['mp4-bytes'], { type: 'video/mp4' })
    })

    const mediaJob = {
      ...job,
      id: 'job-media-1',
      result: { summary: 'Final report text' },
      artifacts: [
        {
          id: 'art-note',
          filename: 'notes.md',
          media_type: 'text/markdown',
          size_bytes: 512,
          sha256: 'a'.repeat(64),
        },
        {
          id: 'art-poster',
          filename: 'poster.png',
          media_type: 'image/png',
          size_bytes: 4096,
          sha256: 'b'.repeat(64),
          download_url: '/api/admin/jobs/job-media-1/artifacts/art-poster',
        },
        {
          id: 'art-demo',
          filename: 'demo.mp4',
          media_type: 'video/mp4',
          size_bytes: 8192,
          sha256: 'c'.repeat(64),
          download_url: '/api/admin/jobs/job-media-1/artifacts/art-demo',
        },
      ],
    } as unknown as AdminJob

    const activities = projectUnifiedActivity(mediaJob, [event(7, 'job_finished', { status: 'succeeded' })])
    const wrapper = mount(JobUnifiedTimeline, {
      props: {
        job: mediaJob,
        jobId: 'job-media-1',
        activities,
        selectedId: null,
        filter: { kind: 'all' },
        roles: [],
      },
    })
    try {
      await flushPromises()

      // Result node is selected by default when job finishes with output
      const inspector = wrapper.get('[data-tree-inspector]')
      expect(inspector.text()).toContain('Final report text')
      expect(inspector.text()).toContain('notes.md')

      const mediaRow = wrapper.get('[data-step="job:media"]')
      expect(mediaRow.attributes('data-kind')).toBe('media')
      await mediaRow.trigger('click')

      expect(inspector.find('[data-media-library-grid]').exists()).toBe(true)
      expect(inspector.find('.detail-heading').exists()).toBe(false)
      expect(inspector.find('.context-line').exists()).toBe(false)
      expect(inspector.find('.feed').exists()).toBe(false)

      const cards = inspector.findAll('[data-media-card]')
      expect(cards).toHaveLength(2)
      expect(inspector.findAll('[data-artifact-download-btn]')).toHaveLength(2)

      expect(inspector.text()).toContain('poster.png')
      expect(inspector.text()).toContain('demo.mp4')

      const img = inspector.get('[data-canvas-media-image]')
      expect(img.attributes('alt')).toBe('poster.png')
      expect(img.attributes('src')).toMatch(/^blob:/)

      const video = inspector.get('[data-canvas-media-video]')
      expect(video.attributes('src')).toMatch(/^blob:/)

      // Clicking card opens preview dialog and reuses preloaded blob URL without a duplicate fetch
      const callsBeforeModal = artifactSpy.mock.calls.length
      await cards[0]!.trigger('click')
      await flushPromises()
      expect(wrapper.find('[data-artifact-detail-modal]').exists()).toBe(true)
      expect(artifactSpy.mock.calls.length).toBe(callsBeforeModal)
    } finally {
      wrapper.unmount()
      artifactSpy.mockRestore()
    }
  })
})

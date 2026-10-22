import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it } from 'vitest'
import type { AdminJob, JobEvent } from '@/api/types/agent-jobs'
import { setLocale } from '@/composables/useI18n'
import { projectUnifiedActivity } from '@/domain/jobs/unified-activity'
import JobUnifiedTimeline from './JobUnifiedTimeline.vue'

const job = { status: 'succeeded', steps: [], agent_runs: [
  { agent_run_id: 'root', role: 'orchestrator', parent_agent_run_id: null },
  { agent_run_id: 'research-a', role: 'research', parent_agent_run_id: 'root', fork_group_id: 'turn', goal_excerpt: 'Read source A' },
  { agent_run_id: 'research-b', role: 'research', parent_agent_run_id: 'root', fork_group_id: 'turn', goal_excerpt: 'Read source B' },
] } as unknown as AdminJob
const event = (sequence: number, event_type: JobEvent['event_type'], payload: Record<string, unknown>): JobEvent => ({
  sequence, event_type, payload, step_key: null, created_at: '2026-10-15T00:00:00Z',
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

describe('nested Job timeline', () => {
  it('groups interleaved children by identity and keeps their finish inside the branch', async () => {
    const activities = projectUnifiedActivity(job, events)
    const wrapper = mount(JobUnifiedTimeline, { props: { activities, selectedId: null, filter: { kind: 'all' }, roles: ['research'] } })
    try {
      const branches = wrapper.findAll('.child-group')
      expect(branches).toHaveLength(2)
      for (const [index, branch] of branches.entries()) {
        expect(branch.get('summary').text()).toContain('Sub-agent · Research')
        expect(branch.text()).toContain(index === 0 ? 'Read source A' : 'Read source B')
        expect(branch.text()).toContain('Agent finished')
        expect(branch.findAll('.timeline-row')).toHaveLength(2)
        const details = branch.get('details').element as HTMLDetailsElement
        expect(details.open).toBe(false)
        branch.get('summary').element.click()
        expect(details.open).toBe(true)
      }
      expect(wrapper.findAll('.timeline-group:not(.child-group)').map(row => row.text()).join(' ')).not.toContain('Agent finished')
      expect(wrapper.text()).toContain('Sub-agents: 2')
      expect(wrapper.text()).toContain('Children: 2')
      const join = wrapper.findAll('button').find(button => button.text().includes('Join completed'))!
      await join.trigger('click')
      expect(wrapper.emitted('select')).toEqual([['join:turn']])
      const role = wrapper.findAll('button').find(button => button.text() === 'Research')!
      await role.trigger('click')
      expect(wrapper.emitted('filter')).toEqual([[{ kind: 'role', role: 'research' }]])
    } finally { wrapper.unmount() }
  })

  it('keeps running agents expanded, collapses when finished and allows reopening across refreshes', async () => {
    const running = { ...job, status: 'running' as const }
    const activeEvents = [...events.slice(0, 3), event(4, 'llm_run_finished', { run_id: 'llm-a', status: 'completed' })]
    const activities = projectUnifiedActivity(running, activeEvents)
    const wrapper = mount(JobUnifiedTimeline, { props: { activities, selectedId: null, filter: { kind: 'all' }, roles: [] } })
    try {
      const branch = wrapper.get('[data-agent-run-id="research-a"]')
      const details = branch.get('details').element as HTMLDetailsElement
      expect(details.open).toBe(true)
      await wrapper.setProps({ activities: projectUnifiedActivity(running, events.slice(0, 5)) })
      expect(details.open).toBe(false)
      branch.get('summary').element.click()
      expect(details.open).toBe(true)
      await wrapper.setProps({ activities: projectUnifiedActivity(job, events) })
      expect(details.open).toBe(true)
    } finally { wrapper.unmount() }
  })

  it.each(['queued', 'running', 'succeeded', 'failed', 'cancelled', 'interrupted'] as const)(
    'uses the authoritative %s Agent Run status before finish events load', status => {
      const view = { ...job, agent_runs: job.agent_runs!.map(run => ({ ...run, status })) }
      const wrapper = mount(JobUnifiedTimeline, { props: {
        activities: projectUnifiedActivity(view, events.slice(0, 3)), selectedId: null, filter: { kind: 'all' }, roles: [],
      } })
      try {
        const details = wrapper.get('.child-group details').element as HTMLDetailsElement
        expect(details.open).toBe(status === 'queued' || status === 'running')
      } finally { wrapper.unmount() }
    },
  )

  it('keeps legacy activity on the main axis', () => {
    const activities = projectUnifiedActivity({ ...job, agent_runs: undefined }, [events[6]!])
    const wrapper = mount(JobUnifiedTimeline, { props: { activities, selectedId: null, filter: { kind: 'all' }, roles: [] } })
    try {
      expect(wrapper.find('.child-group').exists()).toBe(false)
      expect(wrapper.text()).toContain('Job finished')
    } finally { wrapper.unmount() }
  })
})

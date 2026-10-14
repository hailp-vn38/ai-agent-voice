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
  it('applies the theme selection accent only to the current timeline item', async () => {
    const activity = projectUnifiedActivity(view, events(5))[0]!
    const activities = [activity, { ...activity, id: 'second' }]
    const timeline = mount(JobUnifiedTimeline, { props: { activities, selectedId: activity.id, filter: { kind: 'all' }, roles: [] } })
    try {
      const items = timeline.findAll('ol button')
      expect(items[0]!.classes()).toContain('shadow-[inset_3px_0_0_var(--primary)]')
      expect(items[0]!.classes()).toContain('border-primary')
      expect(items[0]!.classes()).not.toContain('ring-1')
      expect(items[1]!.classes()).toContain('bg-surface')
      await items[1]!.trigger('click')
      expect(timeline.emitted('select')).toEqual([['second']])
      await timeline.setProps({ selectedId: 'second' })
      expect(items[0]!.attributes('aria-current')).toBeUndefined()
      expect(items[0]!.classes()).not.toContain('shadow-[inset_3px_0_0_var(--primary)]')
      expect(items[1]!.attributes('aria-current')).toBe('true')
      expect(items[1]!.classes()).toContain('shadow-[inset_3px_0_0_var(--primary)]')
    } finally { timeline.unmount() }
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
})

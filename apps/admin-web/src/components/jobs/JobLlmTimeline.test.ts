import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import type { AdminJob } from '@/api/types/agent-jobs'
import { setLocale, translate } from '@/composables/useI18n'
import JobLlmTimeline from './JobLlmTimeline.vue'

const job: AdminJob = {
  id: 'job', goal: 'Work', status: 'waiting', created_at: '2026-10-31T00:00:00Z',
  requester_scope: 'admin', plan_revision: 1, retry_of_job_id: null, result: null,
  budget_used: {}, steps: [], artifacts: [],
}
beforeEach(() => { setLocale('en'); vi.useFakeTimers(); vi.setSystemTime(new Date('2026-10-31T00:02:00Z')) })
afterEach(() => vi.useRealTimers())

it('shows a waiting job as waiting in the elapsed status', () => {
  const wrapper = mount(JobLlmTimeline, { props: { job, events: [] } })
  try { expect(wrapper.text()).toContain(translate('jobUi.jobStatus.waiting')) }
  finally { wrapper.unmount() }
})

it('freezes elapsed time for a budget-stopped job at its finish timestamp', async () => {
  const wrapper = mount(JobLlmTimeline, { props: { job: { ...job, status: 'stopped', finished_at: '2026-10-31T00:01:00Z' }, events: [] } })
  try {
    expect(wrapper.text()).toContain(translate('jobUi.llmFinished'))
    expect(wrapper.text()).toContain('01:00')
    const before = wrapper.text()
    await vi.advanceTimersByTimeAsync(60_000)
    await flushPromises()
    expect(wrapper.text()).toBe(before)
  } finally { wrapper.unmount() }
})

import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { setLocale, translate } from '@/composables/useI18n'
import JobsPage from './JobsPage.vue'
import JobDetailPage from './JobDetailPage.vue'

const { api, listenJobChanges, push } = vi.hoisted(() => ({
  api: { listJobs: vi.fn(), job: vi.fn(), events: vi.fn() },
  listenJobChanges: vi.fn(),
  push: vi.fn(),
}))
vi.mock('@/api/agent-jobs', () => ({ agentJobsApi: api }))
vi.mock('@/api/job-updates', () => ({ listenJobChanges }))
vi.mock('vue-router', () => ({
  useRoute: () => ({ params: { jobId: 'job-1' } }),
  useRouter: () => ({ push }),
}))

beforeEach(() => {
  vi.clearAllMocks()
  vi.useFakeTimers()
  setLocale('en')
  const job = {
    id: 'job-1', goal: 'Run a job', status: 'running',
    created_at: '2026-10-14T00:00:00Z', requester_scope: 'admin',
    plan_revision: 1, retry_of_job_id: null, result: null,
    budget_used: {}, steps: [], artifacts: [],
  }
  api.listJobs.mockResolvedValue({ items: [job], total: 1, page: 1, page_size: 30 })
  api.job.mockResolvedValue(job)
  api.events.mockResolvedValue({ items: [] })
})
afterEach(() => { vi.useRealTimers() })

it.each([
  ['list', JobsPage, api.listJobs],
  ['detail', JobDetailPage, api.job],
] as const)('refreshes the Jobs %s only on user request after the initial load', async (_, component, request) => {
  const wrapper = mount(component, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    expect(request).toHaveBeenCalledTimes(1)
    expect(listenJobChanges).not.toHaveBeenCalled()

    document.dispatchEvent(new Event('visibilitychange'))
    await vi.advanceTimersByTimeAsync(120_000)
    await flushPromises()
    expect(request).toHaveBeenCalledTimes(1)

    await wrapper.findAll('button').find(button => button.text() === translate('jobUi.refresh'))!.trigger('click')
    await flushPromises()
    expect(request).toHaveBeenCalledTimes(2)
    if (component === JobDetailPage) expect(api.events).toHaveBeenCalledTimes(2)
  } finally {
    wrapper.unmount()
  }
})

it('shows a click cursor on both job detail buttons and opens the job', async () => {
  const wrapper = mount(JobsPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    const buttons = wrapper.get('tbody').findAll('button')
    expect(buttons).toHaveLength(2)
    for (const button of buttons) {
      expect(button.classes()).toContain('cursor-pointer')
      await button.trigger('click')
    }
    expect(push).toHaveBeenCalledTimes(2)
    expect(push).toHaveBeenCalledWith({ name: 'job-detail', params: { jobId: 'job-1' } })
  } finally {
    wrapper.unmount()
  }
})


it('loads the full event history during a requested load without scheduling a refresh', async () => {
  let sequence = 0
  api.events.mockImplementation(async () => ({
    items: sequence < 3200 ? Array.from({ length: 100 }, () => ({
      sequence: ++sequence, event_type: 'step_finished',
      created_at: '2026-10-14T00:00:00Z', step_key: null, payload: {},
    })) : [],
  }))
  const wrapper = mount(JobDetailPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    expect(api.events).toHaveBeenCalledTimes(33)
    expect(wrapper.text()).toContain('3200 events')
    await vi.advanceTimersByTimeAsync(120_000)
    expect(api.job).toHaveBeenCalledTimes(1)
    expect(api.events).toHaveBeenCalledTimes(33)
  } finally {
    wrapper.unmount()
  }
})

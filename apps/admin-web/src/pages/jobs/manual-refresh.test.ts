import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { setLocale, translate } from '@/composables/useI18n'
import type { JobChangeCallbacks } from '@/api/job-updates'
import { ApiError } from '@/api/errors'
import JobsPage from './JobsPage.vue'
import JobDetailPage from './JobDetailPage.vue'
import JobUnifiedTimeline from '@/components/jobs/JobUnifiedTimeline.vue'

const { api, listenJobChanges, push } = vi.hoisted(() => ({
  api: { listJobs: vi.fn(), job: vi.fn(), events: vi.fn(), deleteJob: vi.fn(), artifact: vi.fn() },
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
  setActivePinia(createPinia())
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
  api.deleteJob.mockResolvedValue(undefined)
})
afterEach(() => { vi.useRealTimers() })

it.each([false, true])('shows the standalone Job artifacts card (has files: %s)', async hasFiles => {
  const job = await api.job()
  const artifact = { id: 'report.md', media_type: 'text/markdown', size_bytes: 2048, sha256: 'hash' }
  api.job.mockClear().mockResolvedValue({ ...job, artifacts: hasFiles ? [artifact] : [] })
  api.artifact.mockRejectedValue(new ApiError('Download failed', 500, 'artifact_download_failed'))
  const wrapper = mount(JobDetailPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    expect(wrapper.find('details').exists()).toBe(false)
    expect(wrapper.text()).not.toContain(translate('jobUi.technicalDetails'))
    const heading = wrapper.findAll('h3').find(item => item.text() === translate('jobUi.jobArtifacts'))!
    const card = heading.element.closest('section')!
    expect(card.textContent).toContain(hasFiles ? '1' : '0')
    if (hasFiles) {
      expect(card.textContent).toContain(artifact.id)
      expect(card.textContent).toContain('text/markdown · ' + (2048).toLocaleString() + ' B')
      await wrapper.get('button[aria-label="' + translate('jobUi.download') + ' report.md"]').trigger('click')
      await flushPromises()
      expect(api.artifact).toHaveBeenCalledExactlyOnceWith('job-1', 'report.md')
      expect(wrapper.get('[role="alert"]').text()).toBe('artifact_download_failed')
    } else {
      expect(card.textContent).toContain(translate('jobUi.noArtifacts'))
      expect(card.querySelector('button')).toBeNull()
    }
  } finally { wrapper.unmount() }
})

it('shows prototype agent rows with token totals, honest status and timeline filtering', async () => {
  const job = await api.job()
  const run = {
    agent_run_id: 'research-1', parent_agent_run_id: null, role: 'research',
    goal_excerpt: 'Research', status: 'running', run_index: 1, llm_rounds: 4,
    tool_calls: 2, input_tokens: 4000, output_tokens: 1921, started_at: null, finished_at: null,
  }
  api.job.mockClear().mockResolvedValue({ ...job, execution_mode: 'hierarchical_v2', agent_runs: [
    run, { ...run, agent_run_id: 'document-1', role: 'document', run_index: 2, status: 'succeeded', input_tokens: null },
    { ...run, agent_run_id: 'general-1', role: 'general', run_index: 3, status: 'queued' },
  ] })
  const wrapper = mount(JobDetailPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    const heading = wrapper.findAll('h3').find(item => item.text() === translate('jobUi.activeAgents'))!
    const panel = heading.element.closest('section')!
    const rows = wrapper.findAll('button[aria-pressed]').filter(row => panel.contains(row.element))
    expect(rows).toHaveLength(3)
    expect(rows[0]!.get('[aria-hidden="true"]').text()).toBe('R')
    expect(rows[0]!.text()).toContain('4 ' + translate('jobUi.timelineRounds') + ' · ' + (5921).toLocaleString() + ' ' + translate('jobUi.llmTokens'))
    expect(rows[1]!.text()).toContain('— ' + translate('jobUi.llmTokens'))
    expect(rows[1]!.text()).toContain('succeeded')
    expect(rows[2]!.text()).toContain('queued')
    expect(rows[2]!.text()).not.toContain('running')
    await rows[0]!.trigger('click')
    expect(rows[0]!.attributes('aria-pressed')).toBe('true')
    expect(rows[1]!.attributes('aria-pressed')).toBe('false')
    expect(wrapper.text()).toContain(translate('jobUi.agentRunFilter'))
  } finally { wrapper.unmount() }
})

it.each(['queued', 'planning', 'running', 'cancelling'])('hides deletion for a %s Job', async status => {
  const job = await api.job()
  api.job.mockClear().mockResolvedValue({ ...job, status })
  const wrapper = mount(JobDetailPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    expect(wrapper.findAll('button').some(button => button.text() === translate('jobUi.deleteJob'))).toBe(false)
  } finally {
    wrapper.unmount()
  }
})

it.each(['succeeded', 'partial_success', 'failed', 'cancelled'])('confirms deletion of a %s Job before returning to the list', async status => {
  const job = await api.job()
  api.job.mockClear().mockResolvedValue({ ...job, status })
  let finishDelete!: () => void
  api.deleteJob.mockReturnValue(new Promise<void>(resolve => { finishDelete = resolve }))
  const wrapper = mount(JobDetailPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    const deleteButton = wrapper.findAll('button').find(button => button.text() === translate('jobUi.deleteJob'))!
    await deleteButton.trigger('click')
    expect(api.deleteJob).not.toHaveBeenCalled()
    const dialog = wrapper.get('[role="dialog"]')
    expect(dialog.text()).toContain(translate('jobUi.deleteConfirm'))
    await dialog.findAll('button').find(button => button.text() === translate('jobUi.deleteJob'))!.trigger('click')
    expect(api.deleteJob).toHaveBeenCalledExactlyOnceWith('job-1')
    expect(deleteButton.attributes('disabled')).toBeDefined()
    expect(push).not.toHaveBeenCalled()
    finishDelete()
    await flushPromises()
    expect(push).toHaveBeenCalledWith({ name: 'jobs' })
  } finally {
    wrapper.unmount()
  }
})

it('shows deletion conflicts and keeps the Job detail available', async () => {
  const job = await api.job()
  api.job.mockClear().mockResolvedValue({ ...job, status: 'failed' })
  api.deleteJob.mockRejectedValue(new ApiError('Conflict', 409, 'job_delete_conflict'))
  const wrapper = mount(JobDetailPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    await wrapper.findAll('button').find(button => button.text() === translate('jobUi.deleteJob'))!.trigger('click')
    await wrapper.get('[role="dialog"]').findAll('button').find(button => button.text() === translate('jobUi.deleteJob'))!.trigger('click')
    await flushPromises()
    expect(wrapper.get('[role="alert"]').text()).toBe(translate('jobUi.deleteConflict'))
    expect(wrapper.text()).toContain('Run a job')
    expect(push).not.toHaveBeenCalled()
  } finally {
    wrapper.unmount()
  }
})
it('refreshes the Jobs list only on user request after the initial load', async () => {
  const wrapper = mount(JobsPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    expect(api.listJobs).toHaveBeenCalledTimes(1)
    expect(listenJobChanges).not.toHaveBeenCalled()

    document.dispatchEvent(new Event('visibilitychange'))
    await vi.advanceTimersByTimeAsync(120_000)
    await flushPromises()
    expect(api.listJobs).toHaveBeenCalledTimes(1)

    await wrapper.findAll('button').find(button => button.text() === translate('jobUi.refresh'))!.trigger('click')
    await flushPromises()
    expect(api.listJobs).toHaveBeenCalledTimes(2)
  } finally {
    wrapper.unmount()
  }
})

it('refreshes the Jobs detail on user request and relevant SSE hints, then stops on unmount', async () => {
  const event = (sequence: number) => ({
    sequence, event_type: 'step_finished',
    created_at: '2026-10-14T00:00:00Z', step_key: 'step-' + sequence, payload: {},
  })
  api.events
    .mockResolvedValueOnce({ items: [event(1)] })
    .mockResolvedValueOnce({ items: [event(2)] })
  const wrapper = mount(JobDetailPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    expect(api.job).toHaveBeenCalledTimes(1)
    expect(listenJobChanges).toHaveBeenCalledTimes(1)
    expect(wrapper.getComponent(JobUnifiedTimeline).props('activities')).toHaveLength(1)
    const [callbacks, signal] = listenJobChanges.mock.calls[0] as [JobChangeCallbacks, AbortSignal]

    callbacks.onChanged('another-job')
    await vi.advanceTimersByTimeAsync(200)
    expect(api.job).toHaveBeenCalledTimes(1)

    callbacks.onReady()
    callbacks.onChanged('job-1')
    callbacks.onResync()
    await vi.advanceTimersByTimeAsync(200)
    await flushPromises()
    expect(api.job).toHaveBeenCalledTimes(2)
    expect(api.events).toHaveBeenCalledTimes(2)
    expect(api.events).toHaveBeenLastCalledWith('job-1', 1, expect.any(AbortSignal))
    expect(wrapper.getComponent(JobUnifiedTimeline).props('activities')).toHaveLength(2)

    await wrapper.findAll('button').find(button => button.text() === translate('jobUi.refresh'))!.trigger('click')
    await flushPromises()
    expect(api.job).toHaveBeenCalledTimes(3)
    expect(api.events).toHaveBeenCalledTimes(3)
    expect(api.events).toHaveBeenLastCalledWith('job-1', 2, expect.any(AbortSignal))
    expect(wrapper.getComponent(JobUnifiedTimeline).props('activities')).toHaveLength(2)

    callbacks.onChanged('job-1')
    wrapper.unmount()
    expect(signal.aborted).toBe(true)
    await vi.advanceTimersByTimeAsync(120_000)
    expect(api.job).toHaveBeenCalledTimes(3)
  } finally {
    if (wrapper.exists()) wrapper.unmount()
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


it('loads the full event history and keeps the event cursor during fallback refresh', async () => {
  let sequence = 0
  api.events.mockImplementation(async () => ({
    items: sequence < 3200 ? Array.from({ length: 100 }, () => ({
      sequence: ++sequence, event_type: 'step_finished',
      created_at: '2026-10-14T00:00:00Z', step_key: 'step-' + sequence, payload: {},
    })) : [],
  }))
  const wrapper = mount(JobDetailPage, { global: { stubs: { teleport: true, JobUnifiedTimeline: true } } })
  try {
    await flushPromises()
    expect(api.events).toHaveBeenCalledTimes(33)
    expect(wrapper.getComponent(JobUnifiedTimeline).props('activities')).toHaveLength(3200)
    await vi.advanceTimersByTimeAsync(60_200)
    await flushPromises()
    expect(api.job).toHaveBeenCalledTimes(2)
    expect(api.events).toHaveBeenCalledTimes(34)
    expect(api.events).toHaveBeenLastCalledWith('job-1', 3200, expect.any(AbortSignal))
    expect(wrapper.getComponent(JobUnifiedTimeline).props('activities')).toHaveLength(3200)
  } finally {
    wrapper.unmount()
  }
})

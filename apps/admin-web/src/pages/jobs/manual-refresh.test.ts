import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { setLocale, translate } from '@/composables/useI18n'
import type { JobChangeCallbacks } from '@/api/job-updates'
import { ApiError } from '@/api/errors'
import JobsPage from './JobsPage.vue'
import JobDetailPage from './JobDetailPage.vue'

const { api, listenJobChanges, push } = vi.hoisted(() => ({
  api: { listJobs: vi.fn(), job: vi.fn(), events: vi.fn(), deleteJob: vi.fn() },
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

it('shows the selected timeline item details in the Canvas right-side inspector when clicked', async () => {
  const job = await api.job()
  const steps = ['first', 'second'].map(name => ({
    step_key: name, agent_run_id: 'child', plugin_id: 'core', operation: 'web.extract',
    status: 'succeeded', attempt_state: 'finished', instruction: 'Extract ' + name,
    arguments: { url: 'https://example.com/' + name, max_chars: 6000 },
    result: { title: 'Article ' + name, text: 'Extracted news ' + name,
      provenance: [{ source: 'web', urls: ['https://example.com/' + name], capabilities: ['web_extract'] }] },
    required: true, dependencies: [], input_refs: [], output_schema: {},
  }))
  api.job.mockClear().mockResolvedValue({ ...job, steps, agent_runs: [
    { agent_run_id: 'root', role: 'orchestrator', goal_excerpt: 'Coordinate extraction', result: null },
    { agent_run_id: 'child', role: 'general', goal_excerpt: 'Fetch news highlights', result: { summary: 'News summary' } },
  ] })
  api.events.mockResolvedValue({ items: [...steps.map((step, index) => ({
    sequence: index + 1, event_type: 'step_finished', step_key: step.step_key,
    created_at: '2026-10-15T01:16:25Z', payload: { status: 'succeeded', agent_run_id: 'child' },
  })), ...['child', 'root'].map((id, index) => ({
    sequence: index + 3, event_type: 'agent_run_finished', step_key: null,
    created_at: '2026-10-15T01:16:26Z', payload: { status: 'succeeded', agent_run_id: id },
  }))] })
  const wrapper = mount(JobDetailPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    expect(wrapper.find('[data-canvas-inspector]').exists()).toBe(false)
    const items = wrapper.findAll('ol button')
    await items[1]!.trigger('click')
    const inspector = wrapper.get('[data-canvas-inspector]')
    expect(inspector.text()).toContain('Extract second')
    expect(inspector.text()).toContain('https://example.com/second')
    expect(inspector.text()).toContain('Article second')
    expect(inspector.text()).toContain('Extracted news second')
    expect(inspector.text()).toContain('web_extract')
    expect(inspector.text()).not.toContain('Extracted news first')
    await items[0]!.trigger('click')
    expect(wrapper.get('[data-canvas-inspector]').text()).toContain('Extracted news first')
    expect(wrapper.get('[data-canvas-inspector]').text()).not.toContain('Extracted news second')
    await items[2]!.trigger('click')
    expect(wrapper.get('[data-canvas-inspector]').text()).toContain('Fetch news highlights')
    expect(wrapper.get('[data-canvas-inspector]').text()).toContain('News summary')
    await items[3]!.trigger('click')
    expect(wrapper.get('[data-canvas-inspector]').text()).toContain('Coordinate extraction')
    expect(wrapper.get('[data-canvas-inspector]').text()).not.toContain('Extracted news')
    expect(wrapper.get('[data-canvas-inspector]').text()).not.toContain('News summary')
  } finally { wrapper.unmount() }
})

it.each([
  { result: { summary: 'Completed report' }, expected: 'Completed report' },
  { result: { count: 2 }, expected: JSON.stringify({ count: 2 }, null, 2) },
  { result: null, expected: '—' },
])('shows a standalone Result card for $result and artifacts below the canvas', async ({ result, expected }) => {
  const job = await api.job()
  api.job.mockClear().mockResolvedValue({ ...job, result, artifacts: [
    { id: 'report.txt', media_type: 'text/plain', size_bytes: 12, sha256: 'digest' },
  ] })
  const wrapper = mount(JobDetailPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    const heading = wrapper.findAll('h2').find(node => node.text() === translate('jobUi.result'))!
    const card = heading.element.parentElement!
    expect(card.closest('details')).toBeNull()
    expect(card.parentElement).toBe(wrapper.element)
    expect(card.textContent).toContain(expected)
    const artifactsHeading = wrapper.findAll('h2').find(node => node.text() === translate('jobUi.artifacts'))!
    const artifacts = artifactsHeading.element.parentElement!
    expect(artifacts.parentElement).toBe(wrapper.element)
    expect(artifacts.textContent).toContain(translate('jobUi.artifacts'))
    expect(artifacts.textContent).toContain('report.txt')
    expect(artifacts.querySelector('button')?.textContent).toContain(translate('jobUi.download'))
  } finally {
    wrapper.unmount()
  }
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
it('opens Jobs settings from the settings icon', async () => {
  const wrapper = mount(JobsPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    const settings = wrapper.get('button[aria-label="Settings Jobs"]')
    expect(settings.attributes('title')).toBe('Settings Jobs')
    expect(settings.find('svg').exists()).toBe(true)
    await settings.trigger('click')
    expect(push).toHaveBeenCalledWith({ name: 'background' })
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
    created_at: '2026-10-14T00:00:00Z', step_key: null, payload: {},
  })
  api.events
    .mockResolvedValueOnce({ items: [event(1)] })
    .mockResolvedValueOnce({ items: [event(2)] })
  const wrapper = mount(JobDetailPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    expect(api.job).toHaveBeenCalledTimes(1)
    expect(listenJobChanges).toHaveBeenCalledTimes(1)
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

    await wrapper.findAll('button').find(button => button.text() === translate('jobUi.refresh'))!.trigger('click')
    await flushPromises()
    expect(api.job).toHaveBeenCalledTimes(3)
    expect(api.events).toHaveBeenCalledTimes(3)
    expect(api.events).toHaveBeenLastCalledWith('job-1', 2, expect.any(AbortSignal))

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
      created_at: '2026-10-14T00:00:00Z', step_key: null, payload: {},
    })) : [],
  }))
  const wrapper = mount(JobDetailPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    expect(api.events).toHaveBeenCalledTimes(33)
    await vi.advanceTimersByTimeAsync(60_200)
    await flushPromises()
    expect(api.job).toHaveBeenCalledTimes(2)
    expect(api.events).toHaveBeenCalledTimes(34)
    expect(api.events).toHaveBeenLastCalledWith('job-1', 3200, expect.any(AbortSignal))
  } finally {
    wrapper.unmount()
  }
})

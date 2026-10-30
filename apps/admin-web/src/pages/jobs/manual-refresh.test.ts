import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { setLocale, translate } from '@/composables/useI18n'
import type { JobChangeCallbacks } from '@/api/job-updates'
import { ApiError } from '@/api/errors'
import JobsPage from './JobsPage.vue'
import JobDetailPage from './JobDetailPage.vue'

const { api, listenJobChanges, push } = vi.hoisted(() => ({
  api: { listJobs: vi.fn(), job: vi.fn(), events: vi.fn(), deleteJob: vi.fn(), deleteJobs: vi.fn(), createJob: vi.fn() },
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
  api.deleteJobs.mockResolvedValue({ deleted: 0, deleted_ids: [] })
})
afterEach(() => { vi.useRealTimers() })

it('shows the selected execution tree node details in the inline right-hand inspector when clicked', async () => {
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
    const inspector = wrapper.get('[data-tree-inspector]')
    await wrapper.get('[data-node="step:second"]').trigger('click')
    expect(inspector.text()).toContain('Extract second')
    expect(inspector.text()).toContain('https://example.com/second')
    expect(inspector.text()).toContain('Article second')
    expect(inspector.text()).toContain('Extracted news second')
    expect(inspector.text()).toContain('web_extract')
    expect(inspector.text()).not.toContain('Extracted news first')

    await wrapper.get('[data-node="step:first"]').trigger('click')
    expect(inspector.text()).toContain('Extracted news first')
    expect(inspector.text()).not.toContain('Extracted news second')

    await wrapper.get('[data-node="agent:child"]').trigger('click')
    expect(inspector.text()).toContain('Fetch news highlights')
    expect(inspector.text()).toContain('News summary')

    await wrapper.get('[data-node="root"]').trigger('click')
    expect(inspector.text()).toContain('Coordinate extraction')
    expect(inspector.text()).not.toContain('Extracted news')
    expect(inspector.text()).not.toContain('News summary')
  } finally { wrapper.unmount() }
})

it.each([
  { result: { summary: 'Completed report' }, expected: 'Completed report' },
  { result: { count: 2 }, expected: JSON.stringify({ count: 2 }, null, 2) },
  { result: null, expected: 'Job output' },
])('renders Result and artifacts directly inside the Execution Tree inspector workspace instead of standalone panels below', async ({ result, expected }) => {
  const job = await api.job()
  api.job.mockClear().mockResolvedValue({ ...job, result, artifacts: [
    { id: 'report.txt', media_type: 'text/plain', size_bytes: 12, sha256: 'digest' },
  ] })
  const wrapper = mount(JobDetailPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    expect(wrapper.findAll('h2').some(node => node.text() === translate('jobUi.artifacts'))).toBe(false)
    expect(wrapper.find('[data-step="job:result"]').exists()).toBe(true)
    const inspector = wrapper.get('[data-tree-inspector]')
    expect(inspector.text()).toContain(expected)
    expect(inspector.text()).toContain('report.txt')
    expect(inspector.text()).toContain(translate('jobUi.download'))
  } finally {
    wrapper.unmount()
  }
})

it('renders the top stats bar inside the Execution Tree workspace shell', async () => {
  const job = await api.job()
  api.job.mockClear().mockResolvedValue({
    ...job,
    agent_runs: [
      { agent_run_id: 'root', run_index: 0, role: 'orchestrator', status: 'succeeded', input_tokens: 1000, output_tokens: 500, llm_rounds: 1 },
      { agent_run_id: 'child', run_index: 1, role: 'research', status: 'succeeded', input_tokens: 2000, output_tokens: 246, llm_rounds: 1 },
    ],
    steps: [
      {
        step_key: 'step-1',
        plugin_id: 'core',
        operation: 'web.search',
        status: 'succeeded',
        attempt_state: 'finished',
        instruction: 'Search',
        required: true,
        dependencies: [],
        input_refs: [],
        arguments: {},
        output_schema: {},
        result: null,
      },
    ],
  })
  api.events.mockResolvedValue({
    items: [
      { sequence: 1, event_type: 'agent_run_started', step_key: null, created_at: '2026-10-15T00:00:00Z', payload: { agent_run_id: 'root' } },
      { sequence: 2, event_type: 'agent_run_started', step_key: null, created_at: '2026-10-15T00:00:01Z', payload: { agent_run_id: 'child' } },
    ],
  })
  const wrapper = mount(JobDetailPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    const statsBar = wrapper.get('[data-canvas-stats]')
    expect(statsBar.get('[data-canvas-stat="agent-runs"]').text()).toContain('1 root · 1 sub-agents')
    expect(statsBar.get('[data-canvas-stat="llm-runs"]').text()).toContain('2')
    expect(statsBar.get('[data-canvas-stat="plugin-calls"]').text()).toContain('1')
    expect(statsBar.get('[data-canvas-stat="total-tokens"]').text()).toContain((3746).toLocaleString())
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
    expect(push).toHaveBeenCalledWith({ name: 'jobs-settings' })
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

it('shows shimmer on JobDetailPage while the job is loading, then renders the job details', async () => {
  const loadedJob = await api.job()
  let finishJob!: (value: unknown) => void
  api.job.mockReset().mockReturnValue(new Promise(resolve => { finishJob = resolve }))

  const wrapper = mount(JobDetailPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    expect(wrapper.find('[data-job-detail-skeleton]').exists()).toBe(true)
    expect(wrapper.get('[data-job-detail-skeleton]').attributes('aria-busy')).toBe('true')
    expect(wrapper.findAll('.card-shimmer').length).toBeGreaterThan(0)

    finishJob(loadedJob)
    await flushPromises()
    expect(wrapper.find('[data-job-detail-skeleton]').exists()).toBe(false)
    expect(wrapper.text()).toContain('Run a job')
  } finally {
    wrapper.unmount()
  }
})

it('attaches .wav audio in Create Job modal, auto-fills goal, and submits base64 payload', async () => {
  vi.useRealTimers()
  const createdJob = await api.job()
  api.createJob.mockResolvedValue({ ...createdJob, id: 'job-wav-1' })

  const wrapper = mount(JobsPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    await wrapper.findAll('button').find(button => button.text().includes(translate('jobUi.newJob')))!.trigger('click')
    await flushPromises()

    const dropzone = wrapper.get('[data-testid="job-wav-dropzone"]')
    const fakeWav = new File([new Uint8Array([79, 103, 103, 83, 0, 0, 0, 0])], 'fake.wav', {
      type: 'audio/wav',
    })
    await dropzone.trigger('drop', {
      dataTransfer: { files: [fakeWav] },
    })
    await new Promise(resolve => setTimeout(resolve, 25))
    await flushPromises()
    expect(wrapper.text()).toContain(translate('jobUi.wavInvalidFormat'))

    const wavFile = new File([new Uint8Array([82, 73, 70, 70, 36, 0, 0, 0, 87, 65, 86, 69])], 'cuoc-hop.wav', {
      type: 'audio/wav',
    })
    const mp4File = new File([new Uint8Array([0, 0, 0, 24, 102, 116, 121, 112, 105, 115, 111, 109])], 'ban-ghi-hinh.mp4', {
      type: 'video/mp4',
    })
    await dropzone.trigger('drop', {
      dataTransfer: { files: [wavFile, mp4File] },
    })
    await new Promise(resolve => setTimeout(resolve, 25))
    await flushPromises()

    expect(wrapper.text()).toContain('cuoc-hop.wav')
    expect(wrapper.text()).toContain('ban-ghi-hinh.mp4')
    expect((wrapper.get('textarea').element as HTMLTextAreaElement).value).toBe(translate('jobUi.wavDefaultGoal'))

    await wrapper.get('[role="dialog"] form').trigger('submit')
    await flushPromises()

    expect(api.createJob).toHaveBeenCalledTimes(1)
    const [goalArg, keyArg, attachmentsArg] = api.createJob.mock.calls[0]
    expect(goalArg).toBe(translate('jobUi.wavDefaultGoal'))
    expect(typeof keyArg).toBe('string')
    expect(attachmentsArg).toEqual([
      {
        name: 'cuoc-hop.wav',
        mime_type: 'audio/wav',
        data_base64: expect.any(String),
      },
      {
        name: 'ban-ghi-hinh.mp4',
        mime_type: 'video/mp4',
        data_base64: expect.any(String),
      },
    ])
    expect(push).toHaveBeenCalledWith({ name: 'job-detail', params: { jobId: 'job-wav-1' } })
  } finally {
    wrapper.unmount()
  }
})

it('allows selecting multiple Jobs (individual and select-all), confirms batch deletion, and reloads the list', async () => {
  const jobsList = [
    {
      id: 'job-1', goal: 'First job', status: 'running',
      created_at: '2026-10-14T00:00:00Z', requester_scope: 'admin',
      plan_revision: 1, retry_of_job_id: null, result: null,
      budget_used: {}, steps: [], artifacts: [],
    },
    {
      id: 'job-2', goal: 'Second job', status: 'succeeded',
      created_at: '2026-10-14T00:01:00Z', requester_scope: 'admin',
      plan_revision: 1, retry_of_job_id: null, result: null,
      budget_used: {}, steps: [], artifacts: [],
    },
    {
      id: 'job-3', goal: 'Third job', status: 'failed',
      created_at: '2026-10-14T00:02:00Z', requester_scope: 'admin',
      plan_revision: 1, retry_of_job_id: null, result: null,
      budget_used: {}, steps: [], artifacts: [],
    },
  ]
  api.listJobs
    .mockResolvedValueOnce({ items: jobsList, total: 3, page: 1, page_size: 30 })
    .mockResolvedValueOnce({ items: [jobsList[2]!], total: 1, page: 1, page_size: 30 })
  api.deleteJobs.mockResolvedValue({ deleted: 2, deleted_ids: ['job-1', 'job-2'] })

  const wrapper = mount(JobsPage, { global: { stubs: { teleport: true } } })
  try {
    await flushPromises()
    expect(wrapper.find('[data-testid="batch-delete-jobs"]').exists()).toBe(false)

    const selectAll = wrapper.get('thead input[type="checkbox"]')
    const rowCheckboxes = wrapper.findAll('tbody input[type="checkbox"]')
    expect(rowCheckboxes).toHaveLength(3)

    // Toggle select-all ON then OFF
    await selectAll.setValue(true)
    expect(wrapper.get('[data-testid="batch-delete-jobs"]').text()).toContain(
      translate('jobUi.deleteSelectedJobs', { count: 3 }),
    )
    await selectAll.setValue(false)
    expect(wrapper.find('[data-testid="batch-delete-jobs"]').exists()).toBe(false)

    // Select job-1 and job-2 individually
    await rowCheckboxes[0]!.setValue(true)
    await rowCheckboxes[1]!.setValue(true)

    const deleteBtn = wrapper.get('[data-testid="batch-delete-jobs"]')
    expect(deleteBtn.text()).toContain(translate('jobUi.deleteSelectedJobs', { count: 2 }))
    await deleteBtn.trigger('click')
    expect(api.deleteJobs).not.toHaveBeenCalled()

    const dialog = wrapper.get('[role="dialog"]')
    expect(dialog.text()).toContain(translate('jobUi.deleteSelectedConfirm', { count: 2 }))
    const confirmBtn = dialog
      .findAll('button')
      .find(button => button.text().includes(translate('jobUi.deleteSelectedJobs', { count: 2 })))!
    await confirmBtn.trigger('click')
    await flushPromises()

    expect(api.deleteJobs).toHaveBeenCalledExactlyOnceWith(['job-1', 'job-2'])
    expect(api.listJobs).toHaveBeenCalledTimes(2)
    expect(wrapper.find('[data-testid="batch-delete-jobs"]').exists()).toBe(false)
    expect(wrapper.findAll('tbody tr')).toHaveLength(1)
    expect(wrapper.text()).toContain('Third job')
  } finally {
    wrapper.unmount()
  }
})

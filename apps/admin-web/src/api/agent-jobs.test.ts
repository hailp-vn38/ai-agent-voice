import { afterEach, describe, expect, it, vi } from 'vitest'

import { agentJobsApi } from './agent-jobs'

function json(value: unknown) {
  return new Response(JSON.stringify(value), { status: 200, headers: { 'Content-Type': 'application/json' } })
}

describe('Agent Job admin clients', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('loads global settings without an Agent-scoped permission endpoint', async () => {
    const settings = { enabled: true, orchestrator_llm_provider: 'config:planner', worker_llm_provider: 'database:worker', revision: 4 }
    const fetch = vi.fn().mockResolvedValue(json(settings))
    vi.stubGlobal('fetch', fetch)
    await expect(agentJobsApi.settings()).resolves.toEqual(settings)
    expect(fetch.mock.calls[0]?.[0]).toContain('/api/admin/background/settings')
  })

  it('protects Background Settings mutations with If-Match', async () => {
    const fetch = vi.fn().mockResolvedValue(json({ enabled: false, orchestrator_llm_provider: null, worker_llm_provider: null, revision: 8 }))
    vi.stubGlobal('fetch', fetch)
    await agentJobsApi.updateSettings({
      enabled: false,
      orchestrator_llm_provider: 'orchestrator:default',
      worker_llm_provider: 'llm_deepseek',
      agents: {
        wiki_researcher: {
          enabled: true,
          provider_ref: 'llm_deepseek',
          steps: 8,
          allowed_tools: ['skill', 'wiki_query'],
          allowed_skills: ['wiki-query'],
          description: 'Custom researcher',
          system_prompt: 'Prompt body',
        },
      },
    }, 7)
    const init = fetch.mock.calls[0]?.[1] as RequestInit
    expect(init.method).toBe('PUT')
    expect(new Headers(init.headers).get('if-match')).toBe('"7"')
    const sent = JSON.parse(String(init.body))
    expect(sent.agents.wiki_researcher.provider_ref).toBe('llm_deepseek')
    expect(sent.agents.wiki_researcher.steps).toBe(8)
  })

  it('submits Admin-owned Jobs', async () => {
    const fetch = vi.fn().mockImplementation(() => Promise.resolve(json({ id: 'job-new', status: 'queued' })))
    vi.stubGlobal('fetch', fetch)
    await agentJobsApi.createJob('Collect public posts')
    const create = fetch.mock.calls[0]?.[1] as RequestInit
    expect(JSON.parse(String(create.body))).toEqual({ goal: 'Collect public posts', output_format: 'text', allow_partial: false })
  })

  it('downloads a Job artifact with the Admin request boundary', async () => {
    const fetch = vi.fn().mockResolvedValue(new Response('fixture result', { status: 200, headers: { 'Content-Type': 'text/plain' } }))
    vi.stubGlobal('fetch', fetch)
    const artifact = await agentJobsApi.artifact('job 1', 'artifact/1')
    expect(await artifact.text()).toBe('fixture result')
    expect(fetch.mock.calls[0]?.[0]).toContain('/api/admin/jobs/job%201/artifacts/artifact%2F1')
  })
  it('deletes a Job through its encoded URL and accepts an empty 204 response', async () => {
    const fetch = vi.fn().mockResolvedValue(new Response(null, { status: 204 }))
    vi.stubGlobal('fetch', fetch)
    await expect(agentJobsApi.deleteJob('job/1')).resolves.toBeUndefined()
    expect(fetch.mock.calls[0]?.[0]).toContain('/api/admin/jobs/job%2F1')
    expect(fetch.mock.calls[0]?.[1].method).toBe('DELETE')
  })

  it('preserves the deletion conflict returned by the backend', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(
      JSON.stringify({ error: { code: 'job_delete_conflict' } }),
      { status: 409, headers: { 'Content-Type': 'application/json' } },
    )))
    await expect(agentJobsApi.deleteJob('job-1')).rejects.toMatchObject({ status: 409, code: 'job_delete_conflict' })
  })
  it('retries as a linked new Job with an explicit safe-read review and replay key', async () => {
    const fetch = vi.fn().mockResolvedValue(json({ id: 'new', retry_of_job_id: 'old', status: 'queued' }))
    vi.stubGlobal('fetch', fetch)
    await agentJobsApi.retryJob('old', 'retry-once')
    expect(fetch.mock.calls[0]?.[0]).toContain('/api/admin/jobs/old/retry')
    const init = fetch.mock.calls[0]?.[1] as RequestInit
    expect(new Headers(init.headers).get('Idempotency-Key')).toBe('retry-once')
    expect(JSON.parse(String(init.body))).toEqual({ confirm_safe_read_only: true })
  })
  it('reads incremental durable Job events with a per-Job cursor', async () => {
    const fetch = vi.fn().mockResolvedValue(json({ items: [{ sequence: 3, event_type: 'plan_rejected', payload: { code: 'proposal_not_json' } }] }))
    vi.stubGlobal('fetch', fetch)
    const result = await agentJobsApi.events('job 1', 2)
    expect(result.items[0]?.sequence).toBe(3)
    expect(fetch.mock.calls[0]?.[0]).toContain('/api/admin/jobs/job%201/events')
    expect(fetch.mock.calls[0]?.[0]).toContain('after=2')
    expect(fetch.mock.calls[0]?.[0]).toContain('limit=100')
  })

})

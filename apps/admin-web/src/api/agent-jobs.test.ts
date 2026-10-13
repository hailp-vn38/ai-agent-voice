import { afterEach, describe, expect, it, vi } from 'vitest'

import { agentJobsApi } from './agent-jobs'
import { agentPluginsApi } from './agent-plugins'

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
    await agentJobsApi.updateSettings({ enabled: false, orchestrator_llm_provider: null, worker_llm_provider: null }, 7)
    const init = fetch.mock.calls[0]?.[1] as RequestInit
    expect(init.method).toBe('PUT')
    expect(new Headers(init.headers).get('if-match')).toBe('"7"')
  })

  it('uploads code packages as multipart instead of MCP bindings', async () => {
    const fetch = vi.fn().mockResolvedValue(json({ id: 'weibo-crawler', status: 'disabled' }))
    vi.stubGlobal('fetch', fetch)
    await agentPluginsApi.install(new File(['package'], 'plugin.zip', { type: 'application/zip' }))
    expect(fetch.mock.calls[0]?.[0]).toContain('/api/admin/plugins/install')
    const init = fetch.mock.calls[0]?.[1] as RequestInit
    expect(init.method).toBe('POST')
    expect(init.body).toBeInstanceOf(FormData)
    expect((init.body as FormData).get('package')).toBeTruthy()
    expect(new Headers(init.headers).has('content-type')).toBe(false)
  })

  it('uses revision-guarded Activate, never implicit plugin reset', async () => {
    const fetch = vi.fn().mockResolvedValue(json({ id: 'p', status: 'ready', revision: 4 }))
    vi.stubGlobal('fetch', fetch)
    await agentPluginsApi.activate('p', 3)
    expect(fetch.mock.calls[0]?.[0]).toContain('/api/admin/plugins/p/activate')
    expect(new Headers((fetch.mock.calls[0]?.[1] as RequestInit).headers).get('if-match')).toBe('"3"')
  })

  it('approves declared worker permissions explicitly with a revision check', async () => {
    const fetch = vi.fn().mockResolvedValue(json({ id: 'weibo', permissions_approved: true, revision: 6 }))
    vi.stubGlobal('fetch', fetch)
    await agentPluginsApi.approvePermissions('weibo', 5, { network_hosts: ['weibo.com'], scratch_storage: true })
    expect(fetch.mock.calls[0]?.[0]).toContain('/api/admin/plugins/weibo/permissions')
    const init = fetch.mock.calls[0]?.[1] as RequestInit
    expect(init.method).toBe('PUT')
    expect(new Headers(init.headers).get('if-match')).toBe('"5"')
    expect(JSON.parse(String(init.body))).toEqual({ network_hosts: ['weibo.com'], scratch_storage: true })
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
})

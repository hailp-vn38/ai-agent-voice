import { afterEach, describe, expect, it, vi } from 'vitest'

import { agentsApi } from './agents'
import { devicesApi } from './devices'
import { historyApi } from './history'
import { mcpApi } from './mcp'
import { providerAdaptersApi } from './provider-adapters'
import { providersApi } from './providers'
import { systemApi } from './system'
import { templatesApi } from './templates'

function installFetch() {
  const fetch = vi.fn(() => Promise.resolve(new Response('{}', { status: 200, headers: { 'Content-Type': 'application/json' } })))
  vi.stubGlobal('fetch', fetch)
  return fetch
}

function call(fetch: ReturnType<typeof vi.fn>, index: number) {
  return fetch.mock.calls[index] as [string, RequestInit]
}

function expectJsonMutation(init: RequestInit, method: string, body: unknown, revision?: number) {
  const headers = init.headers as Headers
  expect(init.method).toBe(method)
  expect(headers.get('Content-Type')).toBe('application/json')
  expect(init.body).toBe(JSON.stringify(body))
  expect(headers.get('If-Match')).toBe(revision === undefined ? null : `"${revision}"`)
}

describe('Admin resource APIs', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('maps system probes to their text and status endpoints', async () => {
    const fetch = installFetch()
    await Promise.all([systemApi.health(), systemApi.ready(), systemApi.status()])
    expect(call(fetch, 0)[0]).toBe('/health')
    expect((call(fetch, 0)[1].headers as Headers).get('Accept')).toBe('text/plain')
    expect(call(fetch, 1)[0]).toBe('/ready')
    expect(call(fetch, 2)[0]).toBe('/api/admin/system')
  })

  it('encodes agents, relationships, and MCP bindings without leaking raw keys', async () => {
    const fetch = installFetch()
    await agentsApi.list()
    await agentsApi.get('a/b')
    await agentsApi.create({ key: 'a', name: 'Agent' })
    await agentsApi.update('a/b', { enabled: false }, 2)
    await agentsApi.remove('a/b', 3)
    await agentsApi.templates('a/b')
    await agentsApi.assignTemplate('a/b', 't/x', 4)
    await agentsApi.unlinkTemplate('a/b', 't/x', 5)
    await agentsApi.setDefaultTemplate('a/b', 't/x', 6)
    await agentsApi.mcpBindings('a/b')
    await agentsApi.bindMcpServer('a/b', 'm/x', { enabled: true, required: false }, 7)
    await agentsApi.unlinkMcpServer('a/b', 'm/x', 8)

    expect(call(fetch, 0)[0]).toBe('/api/admin/agents?page=1&page_size=50&enabled=true&sort=name')
    expect(call(fetch, 1)[0]).toBe('/api/admin/agents/a%2Fb')
    expectJsonMutation(call(fetch, 2)[1], 'POST', { key: 'a', name: 'Agent' })
    expectJsonMutation(call(fetch, 3)[1], 'PATCH', { enabled: false }, 2)
    expect(call(fetch, 4)[0]).toBe('/api/admin/agents/a%2Fb')
    expect((call(fetch, 4)[1].headers as Headers).get('If-Match')).toBe('"3"')
    expect(call(fetch, 5)[0]).toBe('/api/admin/agents/a%2Fb/templates?page=1&page_size=50')
    expect(call(fetch, 6)[0]).toBe('/api/admin/agents/a%2Fb/templates/t%2Fx')
    expect(call(fetch, 7)[1].method).toBe('DELETE')
    expect(call(fetch, 8)[0]).toBe('/api/admin/agents/a%2Fb/default-template/t%2Fx')
    expect(call(fetch, 8)[1].method).toBe('PUT')
    expect((call(fetch, 8)[1].headers as Headers).get('If-Match')).toBe('"6"')
    expect(call(fetch, 9)[0]).toBe('/api/admin/agents/a%2Fb/mcp-bindings')
    expectJsonMutation(call(fetch, 10)[1], 'PUT', { enabled: true, required: false }, 7)
    expect(call(fetch, 11)[0]).toBe('/api/admin/agents/a%2Fb/mcp-bindings/m%2Fx')
  })

  it('maps device queries and mutations including a false enabled filter', async () => {
    const fetch = installFetch()
    await devicesApi.list({ page: 2, pageSize: 10, enabled: false, sort: '-device_id' })
    await devicesApi.get('dev/1')
    await devicesApi.create({ device_id: 'dev', agent_key: 'a', name: 'Kitchen' })
    await devicesApi.update('dev/1', { template_key: null }, 4)
    await devicesApi.remove('dev/1', 5)
    await devicesApi.claimEnrollment({ code: '000001', agent_key: 'a', name: 'Kitchen' })

    expect(call(fetch, 0)[0]).toBe('/api/admin/devices?page=2&page_size=10&enabled=false&sort=-device_id')
    expect(call(fetch, 1)[0]).toBe('/api/admin/devices/dev%2F1')
    expectJsonMutation(call(fetch, 2)[1], 'POST', { device_id: 'dev', agent_key: 'a', name: 'Kitchen' })
    expectJsonMutation(call(fetch, 3)[1], 'PATCH', { template_key: null }, 4)
    expect((call(fetch, 4)[1].headers as Headers).get('If-Match')).toBe('"5"')
    expect(call(fetch, 5)[0]).toBe('/api/admin/device-enrollments/claim')
    expectJsonMutation(call(fetch, 5)[1], 'POST', { code: '000001', agent_key: 'a', name: 'Kitchen' })
  })

  it('maps template CRUD, paged relationships, and provider bindings', async () => {
    const fetch = installFetch()
    await templatesApi.list({ q: 'vi voice', language: 'vi-VN', enabled: false, sort: 'language' })
    await templatesApi.get('t/x')
    await templatesApi.create({ key: 't', name: 'Vietnamese', language: 'vi-VN', prompt: 'Hi' })
    await templatesApi.update('t/x', { prompt: 'Hello' }, 1)
    await templatesApi.remove('t/x', 2)
    await templatesApi.agents('t/x', 3, 20)
    await templatesApi.providers('t/x')
    await templatesApi.bindProvider('t/x', 'tts', { provider_key: 'p' }, 3)
    await templatesApi.unlinkProvider('t/x', 'tts', 4)

    expect(call(fetch, 0)[0]).toBe('/api/admin/templates?page=1&page_size=50&enabled=false&q=vi+voice&language=vi-VN&sort=language')
    expect(call(fetch, 1)[0]).toBe('/api/admin/templates/t%2Fx')
    expectJsonMutation(call(fetch, 2)[1], 'POST', { key: 't', name: 'Vietnamese', language: 'vi-VN', prompt: 'Hi' })
    expectJsonMutation(call(fetch, 3)[1], 'PATCH', { prompt: 'Hello' }, 1)
    expect(call(fetch, 5)[0]).toBe('/api/admin/templates/t%2Fx/agents?page=3&page_size=20')
    expect(call(fetch, 6)[0]).toBe('/api/admin/templates/t%2Fx/providers')
    expectJsonMutation(call(fetch, 7)[1], 'PUT', { provider_key: 'p' }, 3)
    expect(call(fetch, 8)[1].method).toBe('DELETE')
  })

  it('maps providers, diagnostics and wav input/output to their specific contracts', async () => {
    const fetch = installFetch()
    const wav = new Blob(['wav'], { type: 'audio/wav' })
    await providersApi.list({ type: 'tts', q: 'mai', enabled: false, sort: '-name' })
    await providersApi.get('p/x')
    await providersApi.create({ key: 'p', name: 'Mai', type: 'tts', adapter: 'zero', config_json: {} })
    await providersApi.update('p/x', { enabled: false }, 1)
    await providersApi.remove('p/x', 2)
    await providersApi.templates('p/x', 3, 20)
    await providersApi.capabilities('p/x')
    await providersApi.testVad('p/x')
    await providersApi.testAsr('p/x', { audio: wav })
    await providersApi.testLlm('p/x', { input: 'hello' })
    await providersApi.testTts('p/x', { text: 'xin chào', voice: 'mai' })

    expect(call(fetch, 0)[0]).toBe('/api/admin/providers?page=1&page_size=50&enabled=false&q=mai&type=tts&sort=-name')
    expect(call(fetch, 1)[0]).toBe('/api/admin/providers/p%2Fx')
    expectJsonMutation(call(fetch, 2)[1], 'POST', { key: 'p', name: 'Mai', type: 'tts', adapter: 'zero', config_json: {} })
    expectJsonMutation(call(fetch, 3)[1], 'PATCH', { enabled: false }, 1)
    expect(call(fetch, 5)[0]).toBe('/api/admin/providers/p%2Fx/templates?page=3&page_size=20')
    expect(call(fetch, 6)[0]).toBe('/api/admin/providers/p%2Fx/capabilities')
    expect(call(fetch, 7)[1].method).toBe('POST')
    expect((call(fetch, 8)[1].headers as Headers).get('Content-Type')).toBe('audio/wav')
    expect(call(fetch, 8)[1].body).toBe(wav)
    expectJsonMutation(call(fetch, 9)[1], 'POST', { input: 'hello' })
    expectJsonMutation(call(fetch, 10)[1], 'POST', { text: 'xin chào', voice: 'mai' })
  })

  it('returns the TTS diagnostic as a WAV blob instead of attempting JSON parsing', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('wav', { status: 200, headers: { 'Content-Type': 'audio/wav' } })))

    await expect(providersApi.testTts('tts_mai', { text: 'xin chào' })).resolves.toMatchObject({ type: 'audio/wav', size: 3 })
  })

  it('maps adapter discovery, MCP server CRUD, and history filters/purge', async () => {
    const fetch = installFetch()
    await providerAdaptersApi.list('tts')
    await providerAdaptersApi.get('zero/x')
    await providerAdaptersApi.discoverCapabilities('zero/x', { selection: { model: 'default' } })
    await mcpApi.list({ enabled: false, page: 2, pageSize: 5 })
    await mcpApi.get('m/x')
    await mcpApi.create({ key: 'm', name: 'MCP', url: 'http://mcp', auth: { type: 'none' } })
    await mcpApi.update('m/x', { enabled: false }, 2)
    await mcpApi.remove('m/x', 3)
    await historyApi.list({ session_id: 's', device_id: 1, agent_id: 2, template_id: 3, role: 'assistant', sort: '-sequence' })
    await historyApi.purge({ all: 'all', confirm: 'PURGE_ALL_HISTORY' })

    expect(call(fetch, 0)[0]).toBe('/api/admin/provider-adapters?type=tts')
    expect(call(fetch, 1)[0]).toBe('/api/admin/provider-adapters/zero%2Fx')
    expectJsonMutation(call(fetch, 2)[1], 'POST', { selection: { model: 'default' } })
    expect(call(fetch, 3)[0]).toBe('/api/admin/mcp-servers?page=2&page_size=5&enabled=false')
    expect(call(fetch, 4)[0]).toBe('/api/admin/mcp-servers/m%2Fx')
    expectJsonMutation(call(fetch, 5)[1], 'POST', { key: 'm', name: 'MCP', url: 'http://mcp', auth: { type: 'none' } })
    expectJsonMutation(call(fetch, 6)[1], 'PATCH', { enabled: false }, 2)
    expect((call(fetch, 7)[1].headers as Headers).get('If-Match')).toBe('"3"')
    expect(call(fetch, 8)[0]).toBe('/api/admin/history?page=1&page_size=50&session_id=s&device_id=1&agent_id=2&template_id=3&role=assistant&sort=-sequence')
    expectJsonMutation(call(fetch, 9)[1], 'POST', { all: 'all', confirm: 'PURGE_ALL_HISTORY' })
  })
})

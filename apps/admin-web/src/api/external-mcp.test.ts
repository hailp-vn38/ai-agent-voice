import { afterEach, describe, expect, it, vi } from 'vitest'

import { agentsApi } from './agents'
import { externalToolsApi } from './external-tools'
import { mcpApi } from './mcp'

afterEach(() => vi.unstubAllGlobals())

describe('External MCP Admin contract', () => {
  it('parses redacted MCP auth and the real page shape without inventing total', async () => {
    const body = {
      items: [{
        key: 'weather', name: 'Weather', transport: 'streamable_http',
        url: 'https://example.com/mcp', headers: {},
        auth: { type: 'bearer' },
        connect_timeout_ms: 5000, request_timeout_ms: 30000,
        enabled: true, revision: 2, created_at: 1, updated_at: 1,
      }],
      page: 1, page_size: 50, max_page_size: 200,
    }
    const fetch = vi.fn().mockResolvedValue(new Response(JSON.stringify(body), {
      status: 200, headers: { 'Content-Type': 'application/json' },
    }))
    vi.stubGlobal('fetch', fetch)
    const result = await mcpApi.list()
    expect(result.items[0]?.auth).toEqual({ type: 'bearer' })
    expect(result.total).toBeUndefined()
    expect(fetch.mock.calls[0]?.[0]).toBe('/api/admin/mcp-servers?page=1&page_size=50')
  })

  it('expects binding items and uses Agent revision for the mutation', async () => {
    const fetch = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ items: [{ server_key: 'weather', enabled: true, required: false }] }), { status: 200 }))
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
    vi.stubGlobal('fetch', fetch)
    const bindings = await agentsApi.mcpBindings('home/a')
    expect(bindings.items[0]).toEqual({ server_key: 'weather', enabled: true, required: false })
    await agentsApi.bindMcpServer('home/a', 'weather', { enabled: true, required: false }, 7)
    const [url, init] = fetch.mock.calls[1] as [string, RequestInit]
    expect(url).toBe('/api/admin/agents/home%2Fa/mcp-bindings/weather')
    expect((init.headers as Headers).get('If-Match')).toBe('"7"')
    expect(init.body).toBe(JSON.stringify({ enabled: true, required: false }))
  })

  it('reviews an observed tool with its own approval revision and exact fingerprint', async () => {
    const fetch = vi.fn().mockResolvedValue(new Response(JSON.stringify({ revision: 8 }), { status: 200 }))
    vi.stubGlobal('fetch', fetch)
    const input = {
      server_key: 'weather',
      original_name: 'forecast',
      observed_revision: 3,
      fingerprint: 'a'.repeat(64),
      allowed: true,
      sensitive: false,
    }
    await externalToolsApi.review('home/a', input, 7)
    const [url, init] = fetch.mock.calls[0] as [string, RequestInit]
    expect(url).toBe('/api/admin/agents/home%2Fa/tool-allowlist')
    expect(init.method).toBe('PUT')
    expect((init.headers as Headers).get('If-Match')).toBe('"7"')
    expect(JSON.parse(String(init.body))).toEqual(input)
  })
})

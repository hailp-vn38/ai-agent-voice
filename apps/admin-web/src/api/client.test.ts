import { afterEach, describe, expect, it, vi } from 'vitest'

import { apiUrl, jsonRequest, request, requestBlob, requestJson, requestText, withQuery } from './client'
import { ApiError } from './errors'

describe('HTTP client', () => {
  afterEach(() => {
    sessionStorage.clear()
    vi.unstubAllGlobals()
  })

  it('serializes only defined query values and preserves falsy values', () => {
    expect(withQuery('/devices', { enabled: false, page: 0, ignored: undefined, empty: null })).toBe('/devices?enabled=false&page=0')
  })

  it('normalizes paths and creates JSON requests', () => {
    expect(apiUrl('health')).toBe('/health')
    expect(jsonRequest('POST', { name: 'Mai Chi' })).toEqual({
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{"name":"Mai Chi"}',
    })
  })

  it('merges request headers and adds session authentication plus quoted revision', async () => {
    sessionStorage.setItem('voice-agent-admin-token', 'admin-token')
    const fetch = vi.fn().mockResolvedValue(new Response('{"ok":true}', { status: 200 }))
    vi.stubGlobal('fetch', fetch)

    await request('/api/admin/providers', { headers: { Accept: 'application/json', 'X-Request': 'from-init' } }, {
      headers: { 'X-Trace': 'from-options' }, revision: 7,
    })

    const init = fetch.mock.calls[0][1] as RequestInit
    const headers = init.headers as Headers
    expect(headers.get('Accept')).toBe('application/json')
    expect(headers.get('X-Request')).toBe('from-init')
    expect(headers.get('X-Trace')).toBe('from-options')
    expect(headers.get('Authorization')).toBe('Bearer admin-token')
    expect(headers.get('If-Match')).toBe('"7"')
  })

  it('returns JSON, text, blobs and undefined for successful no-content responses', async () => {
    const fetch = vi.fn()
      .mockResolvedValueOnce(new Response('{"name":"Mai Chi"}', { status: 200 }))
      .mockResolvedValueOnce(new Response('ready', { status: 200 }))
      .mockResolvedValueOnce(new Response('wav', { status: 200, headers: { 'Content-Type': 'audio/wav' } }))
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
    vi.stubGlobal('fetch', fetch)

    await expect(requestJson<{ name: string }>('/json')).resolves.toEqual({ name: 'Mai Chi' })
    await expect(requestText('/text')).resolves.toBe('ready')
    await expect(requestBlob('/audio')).resolves.toMatchObject({ type: 'audio/wav', size: 3 })
    await expect(requestJson<void>('/empty')).resolves.toBeUndefined()
  })

  it('turns non-success responses into structured errors', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify({ error: { code: 'revision_conflict', request_id: 'req-7' } }), { status: 409 })))

    await expect(request('/conflict')).rejects.toEqual(new ApiError('API request failed: revision_conflict', 409, 'revision_conflict', 'req-7'))
  })
})

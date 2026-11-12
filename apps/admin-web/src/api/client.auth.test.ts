import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { request } from './client'
import { setSessionHandler, type SessionEvent } from './session'
import { ApiError } from './errors'
import { loginStatusState } from '@/stores/admin-session'

const STORAGE_KEY = 'voice-agent-admin-token'

describe('HTTP client admin session handling', () => {
  let events: SessionEvent[]

  beforeEach(() => {
    events = []
    setSessionHandler(event => events.push(event))
    localStorage.setItem(STORAGE_KEY, 'stored-token')
  })

  afterEach(() => {
    localStorage.clear()
    setSessionHandler(() => {})
    vi.unstubAllGlobals()
  })

  it('sends the stored Admin Web Token as a Bearer header', async () => {
    const fetch = vi.fn().mockResolvedValue(new Response('{}', { status: 200 }))
    vi.stubGlobal('fetch', fetch)

    await request('/api/admin/devices')

    const headers = (fetch.mock.calls[0][1] as RequestInit).headers as Headers
    expect(headers.get('Authorization')).toBe('Bearer stored-token')
  })

  it('clears the token and signals sign-in on 401', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ error: { code: 'unauthorized', request_id: 'req-1' } }), { status: 401 }),
    ))

    await expect(request('/api/admin/devices')).rejects.toBeInstanceOf(ApiError)

    expect(localStorage.getItem(STORAGE_KEY)).toBeNull()
    expect(events).toEqual([{ kind: 'signed_out' }])
  })

  it('drops the cached login status on 401 so the router reads it again', async () => {
    loginStatusState.value = { passwordLoginEnabled: false, mustChangePassword: false }
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ error: { code: 'unauthorized', request_id: 'req-4' } }), { status: 401 }),
    ))

    await expect(request('/api/admin/devices')).rejects.toBeInstanceOf(ApiError)

    expect(loginStatusState.value).toBeNull()
  })

  it('signals password change on 403 password_change_required and keeps the token', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ error: { code: 'password_change_required', request_id: 'req-2' } }), { status: 403 }),
    ))

    await expect(request('/api/admin/devices')).rejects.toMatchObject({ status: 403, code: 'password_change_required' })

    expect(localStorage.getItem(STORAGE_KEY)).toBe('stored-token')
    expect(events).toEqual([{ kind: 'password_change_required' }])
  })

  it('leaves the session alone for anonymous calls such as login', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ error: { code: 'invalid_password', request_id: 'req-3' } }), { status: 401 }),
    ))

    await expect(request('/api/admin/auth/login', {}, { anonymous: true })).rejects.toBeInstanceOf(ApiError)

    expect(localStorage.getItem(STORAGE_KEY)).toBe('stored-token')
    expect(events).toEqual([])
  })
})

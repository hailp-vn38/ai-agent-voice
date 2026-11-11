import { describe, expect, it } from 'vitest'

import { authRedirect } from './auth-guard'

const devices = { name: 'devices', fullPath: '/devices', query: {} }
const login = { name: 'login', fullPath: '/login', query: {} }
const changePassword = { name: 'change-password', fullPath: '/change-password', query: {} }

const enabled = { passwordLoginEnabled: true, mustChangePassword: false }
const mustChange = { passwordLoginEnabled: true, mustChangePassword: true }
const disabled = { passwordLoginEnabled: false, mustChangePassword: false }

describe('admin router guard', () => {
  it('lets every route through when Password Login is off, without a token', () => {
    expect(authRedirect(devices, { status: disabled, token: null })).toBe(true)
    expect(authRedirect(login, { status: disabled, token: null })).toEqual({ path: '/overview' })
  })

  it('sends a tokenless Admin to the login page and remembers the target route', () => {
    expect(authRedirect(devices, { status: enabled, token: null })).toEqual({
      name: 'login',
      query: { redirect: '/devices' },
    })
  })

  it('keeps the login page reachable without a token', () => {
    expect(authRedirect(login, { status: enabled, token: null })).toBe(true)
  })

  it('forces the password change page while the flag is set, whatever route was asked for', () => {
    expect(authRedirect(devices, { status: mustChange, token: 'tok' })).toEqual({
      name: 'change-password',
      query: { redirect: '/devices' },
    })
    expect(authRedirect(changePassword, { status: mustChange, token: 'tok' })).toBe(true)
  })

  it('does not let a signed-in Admin reach the password change page when the flag is clear', () => {
    expect(authRedirect(changePassword, { status: enabled, token: 'tok' })).toEqual({ path: '/overview' })
  })

  it('lets a signed-in Admin into app routes and bounces the login page back to the target', () => {
    expect(authRedirect(devices, { status: enabled, token: 'tok' })).toBe(true)
    expect(authRedirect({ ...login, query: { redirect: '/jobs' } }, { status: enabled, token: 'tok' })).toEqual({ path: '/jobs' })
    expect(authRedirect(login, { status: enabled, token: 'tok' })).toEqual({ path: '/overview' })
  })

  it('ignores an external redirect target and falls back to the overview', () => {
    expect(authRedirect({ ...login, query: { redirect: 'https://evil.example' } }, { status: enabled, token: 'tok' })).toEqual({ path: '/overview' })
  })
})

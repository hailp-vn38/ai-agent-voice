import type { LocationQueryValue } from 'vue-router'

import type { LoginStatus } from '@/stores/auth'

export interface GuardTarget {
  name?: string | symbol | null
  fullPath: string
  query: Record<string, LocationQueryValue | LocationQueryValue[]>
}

export interface GuardState {
  status: LoginStatus
  token: string | null
}

const DEFAULT_ROUTE = '/overview'

/** Only same-app paths may be restored after sign-in; anything else falls back to the overview. */
export function safeTarget(value: unknown): string {
  if (typeof value !== 'string' || !value.startsWith('/') || value.startsWith('//')) return DEFAULT_ROUTE
  return value
}

/**
 * Decides where an Admin navigation goes. `true` lets it through; a location redirects it.
 * Order: Password Login off (no credential needed), no token, forced password change, signed in.
 */
export function authRedirect(to: GuardTarget, state: GuardState): true | { path: string } | { name: 'login' | 'change-password'; query: { redirect: string } } {
  const isLogin = to.name === 'login'
  const isChangePassword = to.name === 'change-password'
  if (!state.status.passwordLoginEnabled) {
    return isLogin || isChangePassword ? { path: DEFAULT_ROUTE } : true
  }
  if (!state.token) {
    return isLogin ? true : { name: 'login', query: { redirect: to.fullPath } }
  }
  if (state.status.mustChangePassword) {
    return isChangePassword ? true : { name: 'change-password', query: { redirect: to.fullPath } }
  }
  if (isLogin || isChangePassword) {
    return { path: safeTarget(to.query.redirect) }
  }
  return true
}

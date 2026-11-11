import { loginStatusState, writeAdminToken } from '@/stores/admin-session'

import { ApiError, toApiError } from './errors'

export type SessionEvent = { kind: 'signed_out' } | { kind: 'signed_out_by_user' } | { kind: 'password_change_required' }

type SessionHandler = (event: SessionEvent) => void

let handler: SessionHandler = () => {}

/** The router registers how a session event moves the Admin; the client only reports it. */
export function setSessionHandler(next: SessionHandler) {
  handler = next
}

/** Applies the session rules to a failed admin request. Returns true when the request was handled. */
export function applySessionFailure(error: ApiError): boolean {
  if (error.status === 401 || error.code === 'unauthorized') {
    writeAdminToken(null)
    handler({ kind: 'signed_out' })
    return true
  }
  if (error.status === 403 && error.code === 'password_change_required') {
    if (loginStatusState.value) loginStatusState.value = { ...loginStatusState.value, mustChangePassword: true }
    handler({ kind: 'password_change_required' })
    return true
  }
  return false
}

/** For streams opened with fetch: reads the failed response and applies the same session rules. */
export async function handleSessionResponse(response: Response): Promise<boolean> {
  if (response.ok) return false
  return applySessionFailure(await toApiError(response))
}

/** Logging out only clears this browser; the server keeps other sessions valid. */
export function signOut() {
  writeAdminToken(null)
  handler({ kind: 'signed_out_by_user' })
}

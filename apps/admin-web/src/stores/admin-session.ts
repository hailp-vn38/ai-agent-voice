import { ref } from 'vue'

const STORAGE_KEY = 'voice-agent-admin-token'

export interface LoginStatus {
  passwordLoginEnabled: boolean
  mustChangePassword: boolean
}

export function readAdminWebToken(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY) || null
  } catch {
    return null
  }
}

// Module-level state so non-component code (HTTP client, SSE streams) can clear the session
// without an active Pinia instance. The auth store exposes the same refs.
export const adminWebTokenState = ref<string | null>(readAdminWebToken())
export const loginStatusState = ref<LoginStatus | null>(null)

export function writeAdminWebToken(token: string | null) {
  const normalized = token?.trim() || null
  adminWebTokenState.value = normalized
  try {
    if (normalized) localStorage.setItem(STORAGE_KEY, normalized)
    else localStorage.removeItem(STORAGE_KEY)
  } catch {
    // Storage may be blocked; the in-memory token still serves this tab.
  }
}

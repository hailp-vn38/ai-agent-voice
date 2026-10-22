import { defineStore } from 'pinia'
import { ref } from 'vue'

const STORAGE_KEY = 'voice-agent-admin-token'
const configuredAdminToken = import.meta.env.VITE_ADMIN_TOKEN?.trim() || null
let cleared = false

export function readAdminToken(): string | null {
  if (typeof sessionStorage === 'undefined') return null
  return sessionStorage.getItem(STORAGE_KEY) || (cleared ? null : configuredAdminToken)
}

export const useAuthStore = defineStore('auth', () => {
  const adminToken = ref<string | null>(readAdminToken())

  function setAdminToken(token: string | null) {
    const normalized = token?.trim() || null
    cleared = normalized === null
    adminToken.value = normalized
    if (typeof sessionStorage === 'undefined') return
    if (normalized) sessionStorage.setItem(STORAGE_KEY, normalized)
    else sessionStorage.removeItem(STORAGE_KEY)
  }

  return { adminToken, setAdminToken }
})

import { defineStore } from 'pinia'
import { ref } from 'vue'

const STORAGE_KEY = 'voice-agent-admin-token'
const configuredAdminToken = import.meta.env.VITE_ADMIN_TOKEN?.trim() || null

export function readAdminToken(): string | null {
  if (typeof sessionStorage === 'undefined') return null
  return sessionStorage.getItem(STORAGE_KEY) || configuredAdminToken
}

export const useAuthStore = defineStore('auth', () => {
  const adminToken = ref<string | null>(readAdminToken())

  function setAdminToken(token: string | null) {
    const normalized = token?.trim() || null
    adminToken.value = normalized
    if (typeof sessionStorage === 'undefined') return
    if (normalized) sessionStorage.setItem(STORAGE_KEY, normalized)
    else sessionStorage.removeItem(STORAGE_KEY)
  }

  return { adminToken, setAdminToken }
})

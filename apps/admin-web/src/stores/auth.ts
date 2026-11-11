import { defineStore } from 'pinia'
import { computed } from 'vue'

import { adminTokenState, loginStatusState, writeAdminToken, type LoginStatus } from './admin-session'

export { readAdminToken, adminTokenState, type LoginStatus } from './admin-session'

export const useAuthStore = defineStore('auth', () => {
  const adminToken = adminTokenState
  const loginStatus = loginStatusState
  const mustChangePassword = computed(() => loginStatus.value?.mustChangePassword === true)

  function setAdminToken(token: string | null) {
    writeAdminToken(token)
  }

  function setLoginStatus(status: LoginStatus | null) {
    loginStatus.value = status
  }

  return { adminToken, loginStatus, mustChangePassword, setAdminToken, setLoginStatus }
})

import { defineStore } from 'pinia'
import { computed } from 'vue'

import { adminWebTokenState, loginStatusState, writeAdminWebToken, type LoginStatus } from './admin-session'

export { readAdminWebToken, adminWebTokenState, type LoginStatus } from './admin-session'

export const useAuthStore = defineStore('auth', () => {
  const adminWebToken = adminWebTokenState
  const loginStatus = loginStatusState
  const mustChangePassword = computed(() => loginStatus.value?.mustChangePassword === true)

  function setAdminWebToken(token: string | null) {
    writeAdminWebToken(token)
  }

  function setLoginStatus(status: LoginStatus | null) {
    loginStatus.value = status
  }

  return { adminWebToken, loginStatus, mustChangePassword, setAdminWebToken, setLoginStatus }
})

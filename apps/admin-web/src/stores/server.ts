import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { systemApi } from '@/api/system'
import type { SystemStatus } from '@/api/types/system'

type ProbeState = 'idle' | 'checking' | 'online' | 'offline'

export const useServerStore = defineStore('server', () => {
  const health = ref<ProbeState>('idle')
  const readiness = ref<ProbeState>('idle')
  const status = ref<SystemStatus | null>(null)
  const checkedAt = ref<Date | null>(null)
  const lastError = ref<unknown>(null)

  const isOnline = computed(() => health.value === 'online')

  async function checkHealth(signal?: AbortSignal) {
    health.value = 'checking'
    lastError.value = null

    try {
      const result = await systemApi.health(signal)
      health.value = result.trim() === 'ok' ? 'online' : 'offline'
    } catch (error) {
      if (signal?.aborted) return
      health.value = 'offline'
      lastError.value = error
    } finally {
      if (!signal?.aborted) {
        checkedAt.value = new Date()
      }
    }
  }

  async function refreshStatus(signal?: AbortSignal) {
    readiness.value = 'checking'
    lastError.value = null
    const [ready, system] = await Promise.allSettled([systemApi.ready(signal), systemApi.status(signal)])
    if (signal?.aborted) return
    readiness.value = ready.status === 'fulfilled' && ready.value.trim() === 'ready' ? 'online' : 'offline'
    if (system.status === 'fulfilled') {
      status.value = system.value
    } else {
      // Never retain stale admin-only runtime figures after an API failure.
      status.value = null
      lastError.value = system.reason
    }
  }

  async function refresh(signal?: AbortSignal) {
    await Promise.all([checkHealth(signal), refreshStatus(signal)])
  }

  return {
    health,
    readiness,
    status,
    checkedAt,
    lastError,
    isOnline,
    checkHealth,
    refreshStatus,
    refresh,
  }
})

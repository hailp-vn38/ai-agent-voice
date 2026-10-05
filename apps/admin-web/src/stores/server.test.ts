import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

import { useServerStore } from './server'

const systemApi = vi.hoisted(() => ({ health: vi.fn(), ready: vi.fn(), status: vi.fn() }))
vi.mock('@/api/system', () => ({ systemApi }))

describe('server store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.resetAllMocks()
  })

  it('distinguishes live health from readiness and keeps an available status result', async () => {
    systemApi.health.mockResolvedValue('ok\n')
    systemApi.ready.mockResolvedValue('not ready')
    systemApi.status.mockResolvedValue({ status: 'degraded' })
    const store = useServerStore()

    await store.refresh()

    expect(store.health).toBe('online')
    expect(store.readiness).toBe('offline')
    expect(store.status).toEqual({ status: 'degraded' })
    expect(store.isOnline).toBe(true)
    expect(store.checkedAt).toBeInstanceOf(Date)
  })

  it('records a failed status request while retaining the independent readiness result', async () => {
    const failure = new Error('database unavailable')
    systemApi.ready.mockResolvedValue('ready')
    systemApi.status.mockRejectedValue(failure)
    const store = useServerStore()

    await store.refreshStatus()

    expect(store.readiness).toBe('online')
    expect(store.lastError).toBe(failure)
  })
})

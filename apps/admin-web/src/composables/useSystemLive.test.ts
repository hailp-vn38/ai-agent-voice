import { effectScope, nextTick, defineComponent, h } from 'vue'
import { mount as mountComponent } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { SystemLiveSnapshot } from '@/api/types/system'
import {
  resetSystemLiveForTest,
  setSystemLiveTransportForTest,
} from './useSystemLive'
import { useSystemLive } from './useSystemLive'

type PendingTransport = {
  onSnapshot: (value: SystemLiveSnapshot) => void
  signal: AbortSignal
  finish: () => void
  fail: () => void
  unauthorized: () => void
}

function makeSnapshot(sampledAt: string, checkedAt = '2026-11-13T00:00:00.000Z'): SystemLiveSnapshot {
  return {
    v: 1,
    resources: {
      sampled_at: sampledAt,
      sample_interval_ms: 5_000,
      system: { cpu_percent: 42, logical_cores: 8, memory_total_bytes: 100, memory_used_bytes: 50 },
      process: { pid: 1, cpu_percent: 10, rss_bytes: 30 },
      storage: { total_bytes: 200, available_bytes: 100 },
    },
    status: {
      checked_at: checkedAt,
      refresh_interval_ms: 15_000,
      summary: {
        status: 'ready', version: '1.0.0', uptime_seconds: 10,
        database: { enabled: true, status: 'ready' },
        providers: { configured: 1, loaded: 1, stale: 0, failed: 0 },
        sessions: { active: 2 },
      },
    },
  }
}

function makeTransport() {
  const connections: PendingTransport[] = []
  const transport = vi.fn((onSnapshot: PendingTransport['onSnapshot'], signal: AbortSignal) =>
    new Promise<'ended' | 'unauthorized'>((resolve, reject) => {
      const connection: PendingTransport = {
        onSnapshot,
        signal,
        finish: () => resolve('ended'),
        fail: () => reject(new Error('network down')),
        unauthorized: () => resolve('unauthorized'),
      }
      connections.push(connection)
      signal.addEventListener('abort', () => resolve('ended'), { once: true })
    }))
  setSystemLiveTransportForTest(transport)
  return { connections, transport }
}

async function settle() {
  await Promise.resolve()
  await nextTick()
  await Promise.resolve()
}

function mount() {
  const scope = effectScope()
  const value = scope.run(() => useSystemLive())!
  return { scope, value }
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(new Date('2026-11-13T00:00:00.000Z'))
  Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'visible' })
})

afterEach(() => {
  resetSystemLiveForTest()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('useSystemLive', () => {
  it('shares one transport between callers and delays closing after the last leaves', async () => {
    const { connections, transport } = makeTransport()
    const component = defineComponent({ setup: () => { useSystemLive(); return () => h('div') } })
    const first = mountComponent(component)
    const second = mountComponent(component)
    await settle()
    expect(transport).toHaveBeenCalledTimes(1)

    first.unmount()
    second.unmount()
    await vi.advanceTimersByTimeAsync(9_999)
    expect(connections[0]?.signal.aborted).toBe(false)

    const returned = mountComponent(component)
    await vi.advanceTimersByTimeAsync(10_000)
    expect(connections[0]?.signal.aborted).toBe(false)
    expect(transport).toHaveBeenCalledTimes(1)
    returned.unmount()
    await vi.advanceTimersByTimeAsync(10_000)
    expect(connections[0]?.signal.aborted).toBe(true)

    const later = mountComponent(component)
    await settle()
    expect(transport).toHaveBeenCalledTimes(2)
    later.unmount()
  })

  it('aborts while the tab is hidden and reconnects immediately when visible', async () => {
    const { connections, transport } = makeTransport()
    const mounted = mount()
    await settle()
    expect(mounted.value.freshness.value).toEqual({ resources: 'unavailable', status: 'unavailable' })
    Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'hidden' })
    document.dispatchEvent(new Event('visibilitychange'))
    await settle()
    expect(connections[0]?.signal.aborted).toBe(true)
    expect(mounted.value.connection.value).toBe('paused')

    Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'visible' })
    document.dispatchEvent(new Event('visibilitychange'))
    await settle()
    expect(transport).toHaveBeenCalledTimes(2)
    mounted.scope.stop()
  })

  it('backs off from one second to thirty seconds and resets after a snapshot', async () => {
    const { connections, transport } = makeTransport()
    const mounted = mount()
    await settle()
    connections[0]!.fail()
    await settle()
    expect(mounted.value.connection.value).toBe('reconnecting')

    await vi.advanceTimersByTimeAsync(999)
    expect(transport).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(1)
    await settle()
    connections[1]!.fail()
    await settle()
    await vi.advanceTimersByTimeAsync(2_000)
    await settle()
    connections[2]!.fail()
    await settle()
    await vi.advanceTimersByTimeAsync(4_000)
    await settle()
    connections[3]!.fail()
    await settle()
    await vi.advanceTimersByTimeAsync(8_000)
    await settle()
    connections[4]!.fail()
    await settle()
    await vi.advanceTimersByTimeAsync(16_000)
    await settle()
    connections[5]!.fail()
    await settle()
    await vi.advanceTimersByTimeAsync(30_000)
    await settle()
    expect(transport).toHaveBeenCalledTimes(7)

    connections[6]!.onSnapshot(makeSnapshot('2026-11-13T00:00:00.000Z'))
    expect(mounted.value.connection.value).toBe('live')
    connections[6]!.finish()
    await settle()
    await vi.advanceTimersByTimeAsync(999)
    expect(transport).toHaveBeenCalledTimes(7)
    await vi.advanceTimersByTimeAsync(1)
    await settle()
    expect(transport).toHaveBeenCalledTimes(8)
    mounted.scope.stop()
  })

  it('clears data after unauthorized and waits for a new caller before retrying', async () => {
    const { connections, transport } = makeTransport()
    const mounted = mount()
    await settle()
    connections[0]!.onSnapshot(makeSnapshot('2026-11-13T00:00:00.000Z'))
    expect(mounted.value.snapshot.value).not.toBeNull()
    connections[0]!.unauthorized()
    await settle()
    expect(mounted.value.snapshot.value).toBeNull()
    expect(mounted.value.cpuHistory.value).toEqual([])
    expect(mounted.value.connection.value).toBe('paused')
    await vi.advanceTimersByTimeAsync(60_000)
    expect(transport).toHaveBeenCalledTimes(1)
    mounted.scope.stop()

    const nextCaller = mount()
    await settle()
    expect(transport).toHaveBeenCalledTimes(2)
    nextCaller.scope.stop()
  })

  it('marks sections stale after three refresh intervals and keeps only 60 distinct CPU samples', async () => {
    const { connections } = makeTransport()
    const mounted = mount()
    await settle()
    const connection = connections[0]!
    connection.onSnapshot(makeSnapshot('2026-11-13T00:00:00.000Z'))
    expect(mounted.value.freshness.value).toEqual({ resources: 'fresh', status: 'fresh' })
    await vi.advanceTimersByTimeAsync(46_000)
    expect(Date.now()).toBe(new Date('2026-11-13T00:00:46.000Z').getTime())
    expect(mounted.value.freshness.value).toEqual({ resources: 'stale', status: 'stale' })

    const firstSample = Date.parse('2026-11-13T00:00:00.000Z')
    for (let index = 1; index <= 61; index += 1) {
      connection.onSnapshot(makeSnapshot(new Date(firstSample + index * 5_000).toISOString()))
    }
    expect(mounted.value.cpuHistory.value).toHaveLength(60)
    const beforeDuplicate = mounted.value.cpuHistory.value
    connection.onSnapshot(makeSnapshot(new Date(firstSample + 61 * 5_000).toISOString()))
    expect(mounted.value.cpuHistory.value).toBe(beforeDuplicate)
    mounted.scope.stop()
  })
})

import { computed, getCurrentScope, onScopeDispose, readonly, ref, type Ref } from 'vue'

import { listenSystemLive, type SystemLiveTransport } from '@/api/system-live'
import type { SystemLiveSnapshot } from '@/api/types/system'

export type Freshness = 'fresh' | 'stale' | 'unavailable'
export type LiveConnection = 'connecting' | 'live' | 'reconnecting' | 'paused'

const IDLE_CLOSE_MS = 10_000
const MIN_RECONNECT_MS = 1_000
const MAX_RECONNECT_MS = 30_000
const MAX_CPU_SAMPLES = 60

const sharedSnapshot = ref<SystemLiveSnapshot | null>(null)
const sharedCpuHistory = ref<number[]>([])
const sharedConnection = ref<LiveConnection>('paused')
const now = ref(Date.now())

let transport: SystemLiveTransport = listenSystemLive
let callerCount = 0
let generation = 0
let backoffMs = MIN_RECONNECT_MS
let idleTimer: ReturnType<typeof setTimeout> | undefined
let reconnectTimer: ReturnType<typeof setTimeout> | undefined
let staleTimer: ReturnType<typeof setInterval> | undefined
let controller: AbortController | undefined
let haltedForUnauthorized = false
let lastSampledAt: string | null = null
let lastCheckedAt: string | null = null
const receivedAt = ref<{ resources: number | null; status: number | null }>({ resources: null, status: null })
let visibilityListenerInstalled = false

const freshness = computed(() => {
  const current = sharedSnapshot.value
  const live = sharedConnection.value === 'live'
  const received = receivedAt.value
  const section = (available: boolean, lastReceivedAt: number | null, intervalMs: number): Freshness => {
    if (!current || !available || lastReceivedAt === null) return 'unavailable'
    if (!live || now.value - lastReceivedAt > intervalMs * 3) return 'stale'
    return 'fresh'
  }

  return {
    resources: section(
      current?.resources !== null && current?.resources !== undefined,
      received.resources,
      current?.resources?.sample_interval_ms ?? 0,
    ),
    status: section(
      current?.status.summary !== null && current?.status.summary !== undefined,
      received.status,
      current?.status.refresh_interval_ms ?? 0,
    ),
  }
})

function visible() {
  return typeof document === 'undefined' || document.visibilityState !== 'hidden'
}

function startFreshnessClock() {
  if (staleTimer) return
  staleTimer = setInterval(() => { now.value = Date.now() }, 1_000)
}

function stopFreshnessClock() {
  if (staleTimer) clearInterval(staleTimer)
  staleTimer = undefined
}

function clearReconnectTimer() {
  if (reconnectTimer) clearTimeout(reconnectTimer)
  reconnectTimer = undefined
}

function clearLiveData() {
  sharedSnapshot.value = null
  sharedCpuHistory.value = []
  lastSampledAt = null
  lastCheckedAt = null
  receivedAt.value = { resources: null, status: null }
}

function recordSnapshot(value: SystemLiveSnapshot, expectedGeneration: number, signal: AbortSignal) {
  if (expectedGeneration !== generation || signal.aborted) return
  const receivedAtClock = Date.now()
  const sampledAt = value.resources?.sampled_at ?? null
  const checkedAt = value.status.checked_at

  if (sampledAt !== null && sampledAt !== lastSampledAt) {
    lastSampledAt = sampledAt
    receivedAt.value = { ...receivedAt.value, resources: receivedAtClock }
    const cpu = value.resources?.system.cpu_percent
    if (typeof cpu === 'number' && Number.isFinite(cpu)) {
      sharedCpuHistory.value = [...sharedCpuHistory.value, cpu].slice(-MAX_CPU_SAMPLES)
    }
  }
  if (checkedAt !== null && checkedAt !== lastCheckedAt) {
    lastCheckedAt = checkedAt
    receivedAt.value = { ...receivedAt.value, status: receivedAtClock }
  }
  // A null section is unavailable even when its last successful timestamp is retained.
  if (value.resources === null) receivedAt.value = { ...receivedAt.value, resources: null }
  if (value.status.summary === null) receivedAt.value = { ...receivedAt.value, status: null }

  sharedSnapshot.value = value
  backoffMs = MIN_RECONNECT_MS
  sharedConnection.value = 'live'
  now.value = receivedAtClock
}

function stopConnection(nextState: LiveConnection) {
  generation += 1
  clearReconnectTimer()
  controller?.abort()
  controller = undefined
  sharedConnection.value = nextState
}

function scheduleReconnect(expectedGeneration: number) {
  if (expectedGeneration !== generation || callerCount === 0 || haltedForUnauthorized || !visible()) return
  sharedConnection.value = 'reconnecting'
  const delay = backoffMs
  backoffMs = Math.min(backoffMs * 2, MAX_RECONNECT_MS)
  clearReconnectTimer()
  reconnectTimer = setTimeout(() => {
    reconnectTimer = undefined
    if (callerCount > 0 && visible() && !haltedForUnauthorized) openConnection()
  }, delay)
}

function openConnection() {
  if (callerCount === 0 || !visible() || haltedForUnauthorized) {
    if (callerCount > 0 && !visible()) sharedConnection.value = 'paused'
    return
  }
  clearReconnectTimer()
  const currentGeneration = ++generation
  const currentController = new AbortController()
  controller = currentController
  sharedConnection.value = sharedSnapshot.value ? 'reconnecting' : 'connecting'
  startFreshnessClock()

  void transport(value => recordSnapshot(value, currentGeneration, currentController.signal), currentController.signal)
    .then(result => {
      if (currentGeneration !== generation || currentController.signal.aborted) return
      controller = undefined
      if (result === 'unauthorized') {
        haltedForUnauthorized = true
        stopFreshnessClock()
        clearLiveData()
        sharedConnection.value = 'reconnecting'
        return
      }
      scheduleReconnect(currentGeneration)
    })
    .catch(() => {
      if (currentGeneration !== generation || currentController.signal.aborted) return
      controller = undefined
      scheduleReconnect(currentGeneration)
    })
}

function onVisibilityChange() {
  if (!visible()) {
    stopConnection('paused')
    return
  }
  if (callerCount > 0 && !haltedForUnauthorized) openConnection()
}

function installVisibilityListener() {
  if (visibilityListenerInstalled || typeof document === 'undefined') return
  document.addEventListener('visibilitychange', onVisibilityChange)
  visibilityListenerInstalled = true
}

function removeVisibilityListener() {
  if (!visibilityListenerInstalled || typeof document === 'undefined') return
  document.removeEventListener('visibilitychange', onVisibilityChange)
  visibilityListenerInstalled = false
}

function releaseCaller() {
  callerCount = Math.max(0, callerCount - 1)
  if (callerCount !== 0) return
  if (idleTimer) clearTimeout(idleTimer)
  idleTimer = setTimeout(() => {
    idleTimer = undefined
    if (callerCount !== 0) return
    stopConnection('paused')
    stopFreshnessClock()
    removeVisibilityListener()
  }, IDLE_CLOSE_MS)
}

export interface SystemLiveState {
  snapshot: Readonly<Ref<SystemLiveSnapshot | null>>
  cpuHistory: Readonly<Ref<readonly number[]>>
  connection: Readonly<Ref<LiveConnection>>
  freshness: Readonly<Ref<{ resources: Freshness; status: Freshness }>>
}

/** Shared, scope-owned view of the System Live Stream for the current browser tab. */
export function useSystemLive(): SystemLiveState {
  if (!getCurrentScope()) throw new Error('useSystemLive_requires_effect_scope')
  if (idleTimer) clearTimeout(idleTimer)
  idleTimer = undefined
  callerCount += 1
  installVisibilityListener()
  const wasUnauthorized = haltedForUnauthorized
  if (wasUnauthorized) {
    haltedForUnauthorized = false
    backoffMs = MIN_RECONNECT_MS
  }
  if (visible() && wasUnauthorized) openConnection()
  else if (callerCount === 1 && visible() && !controller && !reconnectTimer) openConnection()
  else if (!visible()) sharedConnection.value = 'paused'
  onScopeDispose(releaseCaller)

  return {
    snapshot: readonly(sharedSnapshot),
    cpuHistory: readonly(sharedCpuHistory),
    connection: readonly(sharedConnection),
    freshness: readonly(freshness),
  }
}

/** Used by the reconnect button when the shared connection is already recovering. */
export function reconnectSystemLive() {
  if (callerCount === 0 || sharedConnection.value !== 'reconnecting' || haltedForUnauthorized) return
  backoffMs = MIN_RECONNECT_MS
  stopConnection('reconnecting')
  openConnection()
}

/** Test-only transport seam; callers receive no transport controls from useSystemLive(). */
export function setSystemLiveTransportForTest(next: SystemLiveTransport) {
  if (callerCount !== 0) throw new Error('cannot_replace_system_live_transport_while_subscribed')
  transport = next
}

/** Resets module-level ownership so fake-timer tests remain isolated. */
export function resetSystemLiveForTest() {
  stopConnection('paused')
  if (idleTimer) clearTimeout(idleTimer)
  idleTimer = undefined
  clearReconnectTimer()
  stopFreshnessClock()
  removeVisibilityListener()
  callerCount = 0
  haltedForUnauthorized = false
  backoffMs = MIN_RECONNECT_MS
  transport = listenSystemLive
  clearLiveData()
  sharedConnection.value = 'paused'
  now.value = Date.now()
}

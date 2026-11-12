<script setup lang="ts">
import HeroScene from '@/components/admin/HeroScene.vue'
import {
  Activity,
  ArrowRight,
  ArrowUpRight,
  Bell,
  Bot,
  ChevronRight,
  CircleDot,
  Cpu,
  Database,
  KeyRound,
  Radio,
  Server,
  RefreshCcw,
  ShieldCheck,
  Terminal,
  Workflow,
} from '@lucide/vue'
import { storeToRefs } from 'pinia'
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import { RouterLink } from 'vue-router'

import { assistantApi, type AssistantSettings } from '@/api/assistant'
import { isApiError } from '@/api/errors'
import { systemApi } from '@/api/system'
import type { SystemResources } from '@/api/types/system'
import ResourceStatCard from '@/components/system/ResourceStatCard.vue'
import { useI18n } from '@/composables/useI18n'
import type { MessageKey } from '@/i18n/messages'
import { useServerStore } from '@/stores/server'
import { beginSystemLayout } from '@/router/system-scroll'

const server = useServerStore()
const { health, readiness, status, lastError } = storeToRefs(server)
const { t, formatDateTime } = useI18n()

const resources = ref<SystemResources | null>(null)
const assistantSummary = ref<AssistantSettings | null>(null)
const loading = ref(true)
const error = ref('')
const now = ref(Date.now())
const controller = new AbortController()
const finishInitialLayout = beginSystemLayout()
let inFlight = false
let resourcesTimer: number | undefined
let statusTimer: number | undefined

const stale = computed(() => {
  if (!resources.value) return false
  const sampled = Date.parse(resources.value.sampled_at)
  return !Number.isFinite(sampled) || now.value - sampled > 15_000
})
const current = computed(() => (!error.value && !stale.value ? resources.value : null))
const resourceState = computed(
  () =>
    error.value ||
    (stale.value
      ? t('system.stale')
      : loading.value && !resources.value
        ? t('common.loading')
        : ''),
)
const sampledAtLabel = computed(() => {
  const sampled = resources.value && Date.parse(resources.value.sampled_at)
  return sampled && Number.isFinite(sampled) ? formatDateTime(new Date(sampled)) : '—'
})

type SystemState = 'ready' | 'attention' | 'offline' | 'checking'
const systemState = computed<SystemState>(() => {
  if (health.value === 'offline') return 'offline'
  if (health.value !== 'online' || readiness.value === 'checking') return 'checking'
  const snapshot = status.value
  if (readiness.value !== 'online' || !snapshot || lastError.value) return 'attention'
  const { providers, database } = snapshot
  if (snapshot.status !== 'ready' || (providers.failed ?? 0) > 0 || (providers.stale ?? 0) > 0)
    return 'attention'
  if (providers.configured == null || providers.loaded == null || providers.failed == null || providers.stale == null)
    return 'attention'
  if (providers.loaded < providers.configured) return 'attention'
  if (database.enabled && database.status !== 'ready') return 'attention'
  return 'ready'
})
const stateCopy: Record<SystemState, { title: MessageKey; description: MessageKey }> = {
  ready: { title: 'system.deck.ready', description: 'system.deck.readyDesc' },
  attention: { title: 'system.deck.attention', description: 'system.deck.attentionDesc' },
  offline: { title: 'system.deck.offline', description: 'system.deck.offlineDesc' },
  checking: { title: 'system.deck.checking', description: 'system.deck.checkingDesc' },
}
function probeLabel(probe: string) {
  return probe === 'online' ? t('status.device.online')
    : probe === 'offline' ? t('status.device.offline') : t('system.checking')
}
const uptime = computed(() => {
  const seconds = status.value?.uptime_seconds
  if (seconds == null || !Number.isFinite(seconds) || seconds < 0) return '—'
  const units: [number, MessageKey][] = [
    [86400, 'system.deck.day'], [3600, 'system.deck.hour'],
    [60, 'system.deck.minute'], [1, 'system.deck.second'],
  ]
  let remaining = Math.floor(seconds)
  const parts: string[] = []
  for (const [size, key] of units) {
    const value = Math.floor(remaining / size)
    remaining %= size
    if (value > 0 || (size === 1 && parts.length === 0)) parts.push(`${value}${t(key)}`)
    if (parts.length === 2) break
  }
  return parts.join(' ')
})
const assistantStatLabel = computed(() => {
  if (!assistantSummary.value) return t('common.unknown')
  const stateLabel = t(assistantSummary.value.enabled ? 'system.deck.enabled' : 'system.deck.disabled')
  return `${stateLabel} · ${assistantSummary.value.agent_key || '—'}`
})
const databaseLabel = computed(() => !status.value ? '—' : !status.value.database.enabled
  ? t('system.deck.disabled') : status.value.database.status === 'ready'
    ? t('system.deck.dbReady') : status.value.database.status)
const databaseReady = computed(() => status.value?.database.enabled &&
  status.value.database.status === 'ready')

const moduleGroups = computed(() => [
  {
    number: '02', title: 'system.deck.intelligence' as const, description: 'system.deck.intelligenceDesc' as const,
    modules: [
      { id: 'assistant', testId: 'system-open-assistant', icon: Bot, title: 'system.deck.assistant' as const,
        description: 'system.hub.assistantDesc' as const, detail: assistantStatLabel.value },
      { id: 'mcp', testId: 'system-open-mcp-settings', icon: KeyRound, title: 'system.deck.mcp' as const,
        description: 'system.hub.mcpDesc' as const, detail: '/mcp/knowledge' },
      { id: 'runtime', testId: 'system-open-runtime', icon: Cpu, title: 'system.hub.runtimeTitle' as const,
        description: 'system.hub.runtimeDesc' as const,
        detail: `${t('system.estimatedReservation')}: ${bytes(status.value?.provider_runtime?.accounting.reserved_bytes)}` },
    ],
  },
  {
    number: '03', title: 'system.deck.operations' as const, description: 'system.deck.operationsDesc' as const,
    modules: [
      { id: 'transport', testId: 'system-open-transport', icon: Workflow, title: 'system.deck.transport' as const,
        description: 'system.hub.transportDesc' as const, detail: `Rust / Axum · PID ${current.value?.process.pid ?? '—'}` },
      { id: 'security', testId: 'system-open-security', icon: ShieldCheck, title: 'system.hub.securityTitle' as const,
        description: 'system.hub.securityDesc' as const, detail: '' },
      { id: 'backup', testId: 'system-open-backup', icon: Database, title: 'system.hub.backupTitle' as const,
        description: 'system.hub.backupDesc' as const, detail: 'JSON / SQLite' },
      { id: 'notifications', testId: 'system-open-notification-settings', icon: Bell, title: 'system.deck.notifications' as const,
        description: 'system.hub.notificationsDesc' as const, detail: 'Web · Telegram · Zalo' },
    ],
  },
])

function bytes(value: number | null | undefined) {
  if (value == null || !Number.isFinite(value) || value < 0) return '—'
  return value >= 1024 ** 3
    ? `${(value / 1024 ** 3).toFixed(1)} GiB`
    : `${(value / 1024 ** 2).toFixed(1)} MiB`
}
function percent(value: number | null | undefined) {
  return value == null || !Number.isFinite(value) || value < 0 ? '—' : `${value.toFixed(1)}%`
}
function percentage(used: number | null | undefined, total: number | null | undefined) {
  return used == null ||
    total == null ||
    !Number.isFinite(used) ||
    !Number.isFinite(total) ||
    used < 0 ||
    total <= 0
    ? null
    : Math.min(100, Math.max(0, (used / total) * 100))
}
const memoryPercentage = computed(() =>
  percentage(current.value?.system.memory_used_bytes, current.value?.system.memory_total_bytes),
)
const diskUsed = computed(() => {
  const disk = current.value?.storage
  return disk?.total_bytes != null &&
    disk.available_bytes != null &&
    Number.isFinite(disk.total_bytes) &&
    Number.isFinite(disk.available_bytes) &&
    disk.total_bytes >= 0 &&
    disk.available_bytes >= 0
    ? Math.max(0, disk.total_bytes - disk.available_bytes)
    : null
})
const diskPercentage = computed(() => percentage(diskUsed.value, current.value?.storage.total_bytes))

async function refreshResources() {
  if (inFlight || controller.signal.aborted || document.hidden) return
  inFlight = true
  loading.value = true
  try {
    const snapshot = await systemApi.resources(controller.signal)
    if (controller.signal.aborted) return
    resources.value = snapshot
    error.value = ''
  } catch (cause) {
    if (controller.signal.aborted) return
    error.value =
      isApiError(cause) && cause.status === 401
        ? t('system.metricsUnauthorized')
        : t('system.unavailable')
  } finally {
    now.value = Date.now()
    inFlight = false
    loading.value = false
  }
}

async function loadAssistantSummary() {
  try {
    if (!assistantApi.settings) return
    const summary = await assistantApi.settings(controller.signal)
    if (!controller.signal.aborted) assistantSummary.value = summary
  } catch {
    // Non-blocking summary on System Hub
  }
}

function refresh() {
  return Promise.all([server.refresh(controller.signal), refreshResources()])
}
function visible() {
  if (!document.hidden) {
    now.value = Date.now()
    void refreshResources()
  }
}
onMounted(() => {
  void Promise.allSettled([refresh(), loadAssistantSummary()]).then(async () => {
    await nextTick()
    finishInitialLayout()
  })
  resourcesTimer = window.setInterval(() => {
    now.value = Date.now()
    void refreshResources()
  }, 5_000)
  statusTimer = window.setInterval(() => {
    if (!document.hidden) void server.refresh(controller.signal)
  }, 30_000)
  document.addEventListener('visibilitychange', visible)
})
onBeforeUnmount(() => {
  finishInitialLayout()
  controller.abort()
  window.clearInterval(resourcesTimer)
  window.clearInterval(statusTimer)
  document.removeEventListener('visibilitychange', visible)
})
</script>

<template>
  <section class="system-deck" data-testid="system-hub">
    <header class="deck-header hero-surface">
      <div>
        <p class="deck-eyebrow">{{ t('system.deck.eyebrow') }}</p>
        <h1>{{ t('system.deck.title') }}</h1>
        <p class="deck-subtitle">{{ t('system.deck.subtitle') }}</p>
      </div>
      <button
        type="button"
        class="deck-refresh"
        :disabled="loading || health === 'checking' || readiness === 'checking'"
        data-testid="system-refresh"
        @click="refresh"
      >
        <RefreshCcw :class="['size-3.5', loading && 'animate-spin motion-reduce:animate-none']" aria-hidden="true" />
        {{ t('system.refresh') }}
      </button>
      <HeroScene theme="system" surface />
    </header>

    <section class="control-room" :data-state="systemState" data-testid="system-status-strip" :aria-label="t('system.deck.status')">
      <div class="room-heading">
        <span class="room-caption"><span class="signal-mark" aria-hidden="true" />{{ t('system.deck.controlRoom') }}</span>
        <span class="room-version">VELA / {{ status?.version ?? '—' }}</span>
      </div>
      <div class="room-main">
        <div class="room-copy">
          <span class="state-label"><span class="state-dot" />{{ t(`system.deck.label.${systemState}`) }}</span>
          <h2>{{ t(stateCopy[systemState].title) }}</h2>
          <p>{{ t(stateCopy[systemState].description) }}</p>
          <div class="probe-list">
            <div :data-probe="health">
              <Activity aria-hidden="true" /><span>{{ t('system.deck.health') }}</span>
              <strong>{{ probeLabel(health) }}</strong>
            </div>
            <div :data-probe="readiness">
              <CircleDot aria-hidden="true" /><span>{{ t('system.deck.readiness') }}</span>
              <strong>{{ probeLabel(readiness) }}</strong>
            </div>
            <div :data-probe="!status ? 'idle' : databaseReady ? 'online' : status.database.enabled ? 'offline' : 'idle'">
              <Database aria-hidden="true" /><span>{{ t('system.database') }}</span>
              <strong>{{ databaseLabel }}</strong>
            </div>
          </div>
        </div>
        <div class="core-instrument" aria-hidden="true">
          <span class="instrument-coordinate coordinate-top">SYS / 01</span>
          <span class="instrument-coordinate coordinate-bottom">RUST · AXUM</span>
          <svg viewBox="0 0 360 280" fill="none">
            <path d="M180 12V268M22 140H338" class="instrument-grid" />
            <circle cx="180" cy="140" r="117" class="instrument-grid" />
            <circle cx="180" cy="140" r="99" class="instrument-ticks" />
            <circle cx="180" cy="140" r="81" class="instrument-grid" />
            <circle cx="180" cy="140" r="117" class="instrument-arc" stroke-dasharray="145 590" transform="rotate(-90 180 140)" />
            <circle cx="180" cy="140" r="81" class="instrument-inner-arc" stroke-dasharray="115 394" transform="rotate(90 180 140)" />
            <circle cx="180" cy="23" r="4" class="instrument-node" />
            <circle cx="297" cy="140" r="4" class="instrument-node" />
            <path d="M16 16H30M23 9V23M330 264H344M337 257V271" class="instrument-grid" />
          </svg>
          <div class="core-center">
            <Server class="size-7" />
            <strong>{{ t('system.deck.core') }}</strong>
            <span>{{ probeLabel(health) }}</span>
          </div>
        </div>
      </div>
      <div class="room-stats">
        <div><span>{{ t('system.deck.uptime') }}</span><strong data-testid="system-uptime">{{ uptime }}</strong></div>
        <div>
          <span>{{ t('system.deck.providers') }}</span>
          <div class="provider-reading">
            <strong>{{ status?.providers.loaded ?? '—' }}<small> / {{ status?.providers.configured ?? '—' }}</small></strong>
            <RouterLink v-if="(status?.providers.failed ?? 0) > 0 || (status?.providers.stale ?? 0) > 0" to="/system/runtime" class="provider-alert">
              <template v-if="(status?.providers.failed ?? 0) > 0">{{ t('system.deck.failed', { count: status!.providers.failed! }) }}</template>
              <template v-else>{{ t('system.deck.staleProviders', { count: status!.providers.stale! }) }}</template>
              <ArrowUpRight class="size-3" aria-hidden="true" />
            </RouterLink>
          </div>
        </div>
        <div><span>{{ t('system.deck.sessions') }}</span><strong>{{ status?.sessions.active ?? '—' }}<Radio class="session-radio" aria-hidden="true" /></strong></div>
      </div>
    </section>
    <p v-if="lastError" role="status" class="deck-error">{{ t('system.statusUnavailable') }}</p>

    <section class="resource-section" aria-labelledby="resource-heading">
      <header class="section-heading">
        <div class="section-heading-copy">
          <span class="section-index">01</span>
          <div><h2 id="resource-heading">{{ t('system.deck.telemetry') }}</h2><p>{{ t('system.deck.telemetryDesc') }}</p></div>
        </div>
        <RouterLink to="/system/monitor" data-testid="system-open-monitor" class="text-link">
          {{ t('system.hub.openDetail') }}<ArrowUpRight class="size-4" aria-hidden="true" />
        </RouterLink>
      </header>
      <div class="resource-grid">
        <RouterLink to="/system/monitor" class="resource-link">
          <ResourceStatCard
            class="resource-stat"
            :title="t('system.deck.cpu')" :value="percent(current?.system.cpu_percent)"
            :detail="`${t('system.logicalCores')}: ${current?.system.logical_cores ?? '—'}`"
            :percentage="current?.system.cpu_percent"
            :state="resourceState || (current?.system.cpu_percent == null ? t('system.unavailable') : '')"
          />
        </RouterLink>
        <RouterLink to="/system/monitor" class="resource-link">
          <ResourceStatCard
            class="resource-stat"
            :title="t('system.deck.memory')" :value="percent(memoryPercentage)"
            :detail="`${bytes(current?.system.memory_used_bytes)} / ${bytes(current?.system.memory_total_bytes)}`"
            :percentage="memoryPercentage"
            :state="resourceState || (memoryPercentage == null ? t('system.unavailable') : '')"
          />
        </RouterLink>
        <RouterLink to="/system/monitor" class="resource-link">
          <ResourceStatCard
            class="resource-stat"
            :title="t('system.deck.process')" :value="bytes(current?.process.rss_bytes)"
            :detail="`${t('system.processCpu')}: ${percent(current?.process.cpu_percent)}`"
            :state="resourceState || (current?.process.rss_bytes == null ? t('system.unavailable') : '')"
          />
        </RouterLink>
        <RouterLink to="/system/monitor" class="resource-link">
          <ResourceStatCard
            class="resource-stat"
            :title="t('system.deck.storage')" :value="percent(diskPercentage)"
            :detail="`${bytes(diskUsed)} / ${bytes(current?.storage.total_bytes)}`"
            :percentage="diskPercentage"
            :state="resourceState || (diskPercentage == null ? t('system.unavailable') : '')"
          />
        </RouterLink>
      </div>
      <div class="telemetry-footer" :data-live="!!current">
        <span><span class="telemetry-dot" aria-hidden="true" />{{ t(current ? 'system.deck.live' : 'system.deck.waiting') }}</span>
        <span>{{ t('system.deck.interval') }}<span class="footer-separator" aria-hidden="true">/</span>{{ t('system.deck.sampled', { time: sampledAtLabel }) }}</span>
      </div>
    </section>

    <div class="module-groups">
      <section v-for="group in moduleGroups" :key="group.number" class="module-group" :aria-labelledby="`module-heading-${group.number}`">
        <header class="section-heading-copy">
          <span class="section-index">{{ group.number }}</span>
          <div><h2 :id="`module-heading-${group.number}`">{{ t(group.title) }}</h2><p>{{ t(group.description) }}</p></div>
        </header>
        <div class="module-list">
          <RouterLink
            v-for="(module, index) in group.modules" :key="module.id"
            :to="`/system/${module.id}`" :data-system-card="module.id" :data-testid="module.testId"
            class="module-link"
          >
            <span class="module-icon"><component :is="module.icon" class="size-5" aria-hidden="true" /></span>
            <div class="module-copy">
              <h3>{{ t(module.title) }}</h3>
              <p>{{ t(module.description) }}</p>
              <span v-if="module.detail" class="module-detail">{{ module.detail }}</span>
            </div>
            <span class="module-number" aria-hidden="true">0{{ index + 1 }}</span>
            <ChevronRight class="module-arrow size-4" aria-hidden="true" />
          </RouterLink>
        </div>
      </section>
    </div>

    <RouterLink to="/system/logs" class="diagnostics-link" data-system-card="logs" data-testid="system-open-logs">
      <div class="diagnostics-icon" aria-hidden="true"><Terminal class="size-6" /></div>
      <div class="diagnostics-copy">
        <span class="deck-eyebrow">04 / {{ t('system.hub.diagnosticsTitle') }}</span>
        <h2>{{ t('system.deck.diagnostics') }}</h2>
        <p>{{ t('system.deck.diagnosticsDesc') }}</p>
      </div>
      <span class="diagnostics-action">{{ t('system.deck.openConsole') }}<ArrowRight class="size-4" aria-hidden="true" /></span>
    </RouterLink>
  </section>
</template>

<style scoped src="./system-hub.css"></style>

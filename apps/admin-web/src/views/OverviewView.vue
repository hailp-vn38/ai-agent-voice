<script setup lang="ts">
import {
  Activity,
  AlertTriangle,
  ArrowRight,
  ArrowUpRight,
  AudioLines,
  Bot,
  Boxes,
  BrainCircuit,
  CheckCircle2,
  Clock,
  Cpu,
  Database,
  LayoutTemplate,
  Library,
  Mic,
  MonitorSmartphone,
  Plug,
  RefreshCw,
  Server,
  Settings2,
  Sparkles,
  Volume2,
  Workflow,
  XCircle,
} from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref, watch, type Component } from 'vue'
import { RouterLink } from 'vue-router'

import RadialGauge from '@/components/overview/RadialGauge.vue'
import Sparkline from '@/components/overview/Sparkline.vue'
import { systemApi } from '@/api/system'
import type { SystemResources } from '@/api/types/system'
import { useI18n } from '@/composables/useI18n'
import type { MessageKey } from '@/i18n/messages'
import type { ProviderType } from '@/domain/admin'
import { useAdminStore } from '@/stores/admin'
import { useAuthStore } from '@/stores/auth'
import { useServerStore } from '@/stores/server'

const STATUS_INTERVAL_MS = 15_000
const RESOURCES_INTERVAL_MS = 5_000
const CPU_HISTORY = 36

const { t, locale, providerTypeLabel } = useI18n()
const admin = useAdminStore()
const auth = useAuthStore()
const server = useServerStore()

const countsVisible = computed(() => auth.hasAdminAccess && !admin.loading && !admin.error)
const dash = '—'

/* ---------- Live polling ---------- */

const aborter = new AbortController()
const resources = ref<SystemResources | null>(null)
const resourcesFailed = ref(false)
const cpuHistory = ref<number[]>([])
const refreshing = ref(false)
const now = ref(new Date())
let statusTimer: number | undefined
let resourcesTimer: number | undefined
let clockTimer: number | undefined

async function loadResources() {
  if (!auth.hasAdminAccess || aborter.signal.aborted) return
  try {
    const snapshot = await systemApi.resources(aborter.signal)
    resources.value = snapshot
    resourcesFailed.value = false
    const cpu = snapshot.system.cpu_percent
    if (cpu != null && Number.isFinite(cpu)) {
      cpuHistory.value = [...cpuHistory.value, cpu].slice(-CPU_HISTORY)
    }
  } catch {
    if (aborter.signal.aborted) return
    resources.value = null
    resourcesFailed.value = true
  }
}

async function refresh() {
  refreshing.value = true
  try {
    await Promise.all([
      auth.hasAdminAccess ? admin.refreshAll() : Promise.resolve(),
      server.refresh(aborter.signal),
      loadResources(),
    ])
  } finally {
    refreshing.value = false
  }
}

onMounted(() => {
  void server.refresh(aborter.signal)
  void loadResources()
  statusTimer = window.setInterval(() => void server.refresh(aborter.signal), STATUS_INTERVAL_MS)
  resourcesTimer = window.setInterval(() => void loadResources(), RESOURCES_INTERVAL_MS)
  clockTimer = window.setInterval(() => (now.value = new Date()), 60_000)
})

onBeforeUnmount(() => {
  aborter.abort()
  window.clearInterval(statusTimer)
  window.clearInterval(resourcesTimer)
  window.clearInterval(clockTimer)
})

/* ---------- Hero ---------- */

const greeting = computed(() => {
  const hour = now.value.getHours()
  if (hour < 12) return t('overview.greeting.morning')
  if (hour < 18) return t('overview.greeting.afternoon')
  return t('overview.greeting.evening')
})

type SystemState = 'checking' | 'operational' | 'degraded' | 'offline'

// Polls flip readiness to `checking`; keep showing the last verdict meanwhile.
const systemState = ref<SystemState>('checking')
const providerErrors = computed(() => admin.providers.filter((provider) => provider.status === 'error'))
watch(
  () => [server.readiness, server.health, server.status, providerErrors.value.length] as const,
  ([readiness, health, status, errors]) => {
    if (readiness === 'checking' || readiness === 'idle') return
    if (readiness === 'offline' || health === 'offline') systemState.value = 'offline'
    else if ((status?.providers.failed ?? 0) > 0 || errors > 0) systemState.value = 'degraded'
    else systemState.value = 'operational'
  },
  { immediate: true },
)

const stateStyle = computed(() => ({
  checking: { dot: 'bg-white/70', key: 'overview.state.checking' },
  operational: { dot: 'bg-emerald-300', key: 'overview.state.operational' },
  degraded: { dot: 'bg-amber-300', key: 'overview.state.degraded' },
  offline: { dot: 'bg-rose-300', key: 'overview.state.offline' },
}[systemState.value] as { dot: string; key: MessageKey }))

const activeSessions = computed(() => server.status?.sessions.active ?? null)

function formatUptime(seconds: number) {
  const days = Math.floor(seconds / 86_400)
  const hours = Math.floor((seconds % 86_400) / 3_600)
  const minutes = Math.floor((seconds % 3_600) / 60)
  if (days) return `${days}d ${hours}h`
  if (hours) return `${hours}h ${minutes}m`
  return `${minutes}m ${Math.floor(seconds % 60)}s`
}

const lastChecked = computed(() =>
  server.checkedAt
    ? server.checkedAt.toLocaleTimeString(locale.value === 'vi' ? 'vi-VN' : 'en-US', { hour: '2-digit', minute: '2-digit', second: '2-digit' })
    : dash,
)

// Deterministic bar heights so the waveform looks organic without random jitter per render.
const waveBars = Array.from({ length: 48 }, (_, index) => {
  const shape = Math.abs(Math.sin(index * 0.55) * 0.6 + Math.sin(index * 1.7) * 0.4)
  return { height: 18 + shape * 82, delay: (index % 12) * 0.09 }
})

/* ---------- Stat cards ---------- */

const enabledDevices = computed(() => admin.devices.filter((device) => device.status === 'online').length)
const providerBreakdown = computed(() => {
  const counts = { ready: 0, disabled: 0, error: 0 }
  for (const provider of admin.providers) counts[provider.status] += 1
  return counts
})
const templateLanguages = computed(() => [...new Set(admin.templates.map((template) => template.language).filter(Boolean))].slice(0, 2))

function share(part: number, total: number) {
  return total > 0 ? `${(part / total) * 100}%` : '0%'
}

/* ---------- Pipeline ---------- */

const pipelineStages: { type: ProviderType; icon: Component; hint: MessageKey }[] = [
  { type: 'vad', icon: AudioLines, hint: 'overview.pipeline.vad' },
  { type: 'asr', icon: Mic, hint: 'overview.pipeline.asr' },
  { type: 'llm', icon: BrainCircuit, hint: 'overview.pipeline.llm' },
  { type: 'tts', icon: Volume2, hint: 'overview.pipeline.tts' },
]

const pipeline = computed(() =>
  pipelineStages.map((stage) => {
    const providers = admin.providers.filter((provider) => provider.type === stage.type)
    const ready = providers.filter((provider) => provider.status === 'ready')
    return {
      ...stage,
      total: providers.length,
      ready: ready.length,
      lead: ready[0]?.name ?? providers[0]?.name ?? null,
      tone: ready.length > 0 ? 'ready' : providers.length > 0 ? 'warning' : 'empty',
    }
  }),
)

/* ---------- Resources ---------- */

function percentage(part: number | null | undefined, total: number | null | undefined) {
  return part != null && total != null && total > 0 ? (part / total) * 100 : null
}

function bytes(value: number | null | undefined) {
  if (value == null || !Number.isFinite(value)) return dash
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let size = value
  let unit = 0
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024
    unit += 1
  }
  return `${size.toFixed(size >= 10 || unit === 0 ? 0 : 1)} ${units[unit]}`
}

const gauges = computed(() => {
  const snapshot = resources.value
  const storageUsed = snapshot?.storage.total_bytes != null && snapshot.storage.available_bytes != null
    ? Math.max(0, snapshot.storage.total_bytes - snapshot.storage.available_bytes)
    : null
  return [
    {
      key: 'cpu',
      label: t('overview.resources.cpu'),
      value: snapshot?.system.cpu_percent ?? null,
      detail: snapshot?.system.logical_cores != null ? t('overview.resources.cores', { count: snapshot.system.logical_cores }) : undefined,
    },
    {
      key: 'memory',
      label: t('overview.resources.memory'),
      value: percentage(snapshot?.system.memory_used_bytes, snapshot?.system.memory_total_bytes),
      detail: snapshot ? `${bytes(snapshot.system.memory_used_bytes)} / ${bytes(snapshot.system.memory_total_bytes)}` : undefined,
    },
    {
      key: 'storage',
      label: t('overview.resources.storage'),
      value: percentage(storageUsed, snapshot?.storage.total_bytes),
      detail: snapshot ? `${bytes(storageUsed)} / ${bytes(snapshot.storage.total_bytes)}` : undefined,
    },
  ]
})

const processLine = computed(() => {
  const process = resources.value?.process
  if (!process) return null
  const cpu = process.cpu_percent != null ? `${process.cpu_percent.toFixed(1)}%` : dash
  return t('overview.resources.process', { cpu, rss: bytes(process.rss_bytes) })
})

/* ---------- Health ---------- */

type Tone = 'ok' | 'warn' | 'bad' | 'muted'

const healthRows = computed(() => {
  const status = server.status
  const probe = (state: string): Tone => (state === 'online' ? 'ok' : state === 'offline' ? 'bad' : 'muted')
  const count = (value: number | null | undefined, badWhenPositive: boolean): Tone =>
    value == null ? 'muted' : badWhenPositive && value > 0 ? 'bad' : 'ok'
  const db = status?.database
  const rows: { icon: Component; label: MessageKey; value: string; tone: Tone }[] = [
    {
      icon: Server,
      label: 'overview.health.api',
      value: server.health === 'online' ? t('overview.online') : server.health === 'offline' ? t('overview.offline') : dash,
      tone: probe(server.health),
    },
    {
      icon: Activity,
      label: 'overview.health.readiness',
      value: server.readiness === 'online' ? t('overview.ready') : server.readiness === 'offline' ? t('overview.notReady') : dash,
      tone: probe(server.readiness),
    },
    {
      icon: Database,
      label: 'overview.health.database',
      value: db ? db.status : dash,
      tone: !db ? 'muted' : /^(ok|ready|healthy|connected|up)$/i.test(db.status) ? 'ok' : db.enabled ? 'warn' : 'muted',
    },
    {
      icon: Boxes,
      label: 'overview.health.providers',
      value: status?.providers.loaded != null
        ? `${status.providers.loaded}${status.providers.configured != null ? ` / ${status.providers.configured}` : ''}`
        : dash,
      tone: count(status?.providers.loaded, false),
    },
    {
      icon: Clock,
      label: 'overview.health.stale',
      value: status?.providers.stale != null ? String(status.providers.stale) : dash,
      tone: status?.providers.stale ? 'warn' : count(status?.providers.stale, false),
    },
    {
      icon: XCircle,
      label: 'overview.health.failed',
      value: status?.providers.failed != null ? String(status.providers.failed) : dash,
      tone: count(status?.providers.failed, true),
    },
  ]
  const accounting = status?.provider_runtime?.accounting
  if (accounting) {
    rows.push({
      icon: Cpu,
      label: 'overview.health.leases',
      value: `${accounting.active_leases} · ${bytes(accounting.reserved_bytes)}`,
      tone: 'ok',
    })
  }
  return rows
})

const toneDot: Record<Tone, string> = {
  ok: 'bg-success',
  warn: 'bg-warning',
  bad: 'bg-danger',
  muted: 'bg-muted-foreground/40',
}

/* ---------- Attention ---------- */

interface AttentionItem {
  id: string
  tone: 'bad' | 'warn' | 'info'
  text: string
  to: string
}

const attention = computed<AttentionItem[]>(() => {
  const items: AttentionItem[] = []
  if (server.readiness === 'offline') {
    items.push({ id: 'readiness', tone: 'bad', text: t('overview.attention.readiness'), to: '/system' })
  }
  const failed = server.status?.providers.failed ?? 0
  if (failed > 0) items.push({ id: 'failed', tone: 'bad', text: t('overview.attention.providersFailed', { count: failed }), to: '/providers' })
  for (const provider of providerErrors.value) {
    items.push({ id: `provider-${provider.id}`, tone: 'bad', text: t('overview.attention.providerError', { name: provider.name }), to: `/providers/${encodeURIComponent(provider.id)}` })
  }
  const stale = server.status?.providers.stale ?? 0
  if (stale > 0) items.push({ id: 'stale', tone: 'warn', text: t('overview.attention.providersStale', { count: stale }), to: '/providers' })
  if (countsVisible.value) {
    for (const stage of pipeline.value) {
      if (stage.ready === 0) {
        items.push({ id: `stage-${stage.type}`, tone: 'warn', text: t('overview.attention.stageMissing', { stage: stage.type.toUpperCase() }), to: '/providers' })
      }
    }
    for (const agent of admin.agents) {
      if (admin.devicesForAgent(agent.id).length === 0) {
        items.push({ id: `agent-${agent.id}`, tone: 'info', text: t('overview.attention.agentNoDevice', { name: agent.name }), to: `/agents/${encodeURIComponent(agent.id)}` })
      }
    }
  }
  return items
})
const ATTENTION_LIMIT = 5
const visibleAttention = computed(() => attention.value.slice(0, ATTENTION_LIMIT))

const attentionTone = {
  bad: { icon: XCircle, class: 'text-danger bg-danger/10' },
  warn: { icon: AlertTriangle, class: 'text-warning bg-warning/10' },
  info: { icon: Sparkles, class: 'text-studio-violet bg-primary/10' },
} as const

/* ---------- Agents ---------- */

const AGENT_LIMIT = 5
const roster = computed(() =>
  admin.agents.slice(0, AGENT_LIMIT).map((agent) => {
    const devices = admin.devicesForAgent(agent.id)
    return {
      id: agent.id,
      name: agent.name,
      initials: agent.name.split(/\s+/).map((part) => part[0]).join('').slice(0, 2).toUpperCase() || '?',
      hue: [...agent.id].reduce((hash, char) => (hash * 31 + char.charCodeAt(0)) % 360, 7),
      template: admin.getDefaultTemplate(agent.id)?.name ?? null,
      devices: devices.length,
      enabled: devices.filter((device) => device.status === 'online').length,
    }
  }),
)

/* ---------- Quick actions ---------- */

const quickActions: { to: string; label: MessageKey; hint: MessageKey; icon: Component }[] = [
  { to: '/agents', label: 'nav.agents', hint: 'overview.quick.agents', icon: Bot },
  { to: '/templates', label: 'nav.templates', hint: 'overview.quick.templates', icon: LayoutTemplate },
  { to: '/devices', label: 'nav.devices', hint: 'overview.quick.devices', icon: MonitorSmartphone },
  { to: '/providers', label: 'nav.providers', hint: 'overview.quick.providers', icon: Boxes },
  { to: '/knowledge', label: 'nav.knowledge', hint: 'overview.quick.knowledge', icon: Library },
  { to: '/jobs', label: 'nav.jobs', hint: 'overview.quick.jobs', icon: Workflow },
  { to: '/mcp', label: 'nav.mcp', hint: 'overview.quick.mcp', icon: Plug },
  { to: '/system', label: 'nav.system', hint: 'overview.quick.system', icon: Settings2 },
]
</script>

<template>
  <section class="space-y-6">
    <!-- Hero -->
    <header class="overview-hero" aria-labelledby="overview-title">
      <div class="overview-hero-grid pointer-events-none absolute inset-0" aria-hidden="true" />
      <div
        class="overview-wave-field"
        :class="activeSessions ? 'overview-wave-live' : ''"
        aria-hidden="true"
      >
        <span
          v-for="(bar, index) in waveBars"
          :key="index"
          class="overview-wave-bar flex-1 rounded-full bg-white"
          :style="{ height: `${bar.height}%`, animationDelay: `${bar.delay}s` }"
        />
      </div>

      <div class="overview-hero-row">
        <div class="overview-hero-copy">
          <div class="overview-eyebrow-row">
            <p class="text-xs font-semibold uppercase tracking-[0.2em] text-white/70">{{ t('overview.eyebrow') }}</p>
            <div class="overview-health" role="status">
              <span class="relative flex size-1.5">
                <span v-if="systemState !== 'checking'" class="absolute inline-flex size-full animate-ping rounded-full opacity-75 motion-reduce:animate-none" :class="stateStyle.dot" />
                <span class="relative inline-flex size-1.5 rounded-full" :class="stateStyle.dot" />
              </span>
              {{ t(stateStyle.key) }}
            </div>
          </div>
          <h1 id="overview-title">
            {{ greeting }} <span aria-hidden="true">👋</span>
          </h1>
          <p class="overview-hero-intro">{{ t('overview.description') }}</p>
        </div>

        <div class="overview-hero-actions">
          <button
            type="button"
            class="inline-flex h-9 items-center gap-2 rounded-full border border-white/30 bg-white/10 px-3.5 text-sm font-medium backdrop-blur transition hover:bg-white/20 disabled:opacity-60"
            :disabled="refreshing || !auth.hasAdminAccess"
            @click="refresh()"
          >
            <RefreshCw class="size-4" :class="refreshing ? 'animate-spin' : ''" aria-hidden="true" />
            {{ t('common.refresh') }}
          </button>
          <p class="overview-last-checked">
            {{ t('overview.lastChecked', { time: lastChecked }) }} · {{ t('overview.autoRefresh', { seconds: STATUS_INTERVAL_MS / 1000 }) }}
          </p>
        </div>
      </div>
      <dl class="overview-hero-stats">
        <div>
          <dt>{{ t('overview.liveSessions') }}</dt>
          <dd>{{ activeSessions ?? dash }}</dd>
        </div>
        <div>
          <dt>{{ t('overview.uptime') }}</dt>
          <dd>{{ server.status ? formatUptime(server.status.uptime_seconds) : dash }}</dd>
        </div>
        <div>
          <dt>{{ t('overview.version') }}</dt>
          <dd>{{ server.status?.version ?? dash }}</dd>
        </div>
      </dl>
    </header>

    <!-- Stat cards -->
    <div class="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
      <RouterLink to="/agents" class="overview-card group">
        <div class="flex items-center justify-between">
          <p class="text-sm font-medium text-muted-foreground">{{ t('nav.agents') }}</p>
          <span class="overview-card-icon"><Bot class="size-5" aria-hidden="true" /></span>
        </div>
        <p class="mt-4 text-3xl font-semibold tracking-tight tabular-nums">{{ countsVisible ? admin.agents.length : dash }}</p>
        <div class="mt-3 flex items-center justify-between gap-2">
          <p class="text-xs text-muted-foreground">{{ t('overview.registered') }}</p>
          <div v-if="countsVisible && roster.length" class="flex -space-x-2">
            <span
              v-for="agent in roster.slice(0, 4)"
              :key="agent.id"
              class="flex size-6 items-center justify-center rounded-full text-[10px] font-semibold text-white ring-2 ring-card"
              :style="{ background: `hsl(${agent.hue} 65% 52%)` }"
              :title="agent.name"
            >{{ agent.initials }}</span>
          </div>
        </div>
      </RouterLink>

      <RouterLink to="/templates" class="overview-card group">
        <div class="flex items-center justify-between">
          <p class="text-sm font-medium text-muted-foreground">{{ t('nav.templates') }}</p>
          <span class="overview-card-icon"><LayoutTemplate class="size-5" aria-hidden="true" /></span>
        </div>
        <p class="mt-4 text-3xl font-semibold tracking-tight tabular-nums">{{ countsVisible ? admin.templates.length : dash }}</p>
        <div class="mt-3 flex items-center justify-between gap-2">
          <p class="min-w-0 truncate text-xs text-muted-foreground">{{ t('overview.configurations') }}</p>
          <div v-if="countsVisible" class="flex shrink-0 gap-1">
            <span v-for="language in templateLanguages" :key="language" class="max-w-16 truncate rounded-md bg-surface px-1.5 py-0.5 text-[10px] font-semibold uppercase text-muted-foreground" :title="language">{{ language }}</span>
          </div>
        </div>
      </RouterLink>

      <RouterLink to="/devices" class="overview-card group">
        <div class="flex items-center justify-between">
          <p class="text-sm font-medium text-muted-foreground">{{ t('nav.devices') }}</p>
          <span class="overview-card-icon"><MonitorSmartphone class="size-5" aria-hidden="true" /></span>
        </div>
        <p class="mt-4 text-3xl font-semibold tracking-tight tabular-nums">{{ countsVisible ? admin.devices.length : dash }}</p>
        <div class="mt-3 space-y-1.5">
          <div class="flex h-1.5 overflow-hidden rounded-full bg-muted">
            <div class="bg-success transition-all duration-700" :style="{ width: countsVisible ? share(enabledDevices, admin.devices.length) : '0%' }" />
          </div>
          <p class="text-xs text-muted-foreground">
            {{ countsVisible ? t('overview.devicesEnabled', { enabled: enabledDevices, disabled: admin.devices.length - enabledDevices }) : dash }}
          </p>
        </div>
      </RouterLink>

      <RouterLink to="/providers" class="overview-card group">
        <div class="flex items-center justify-between">
          <p class="text-sm font-medium text-muted-foreground">{{ t('nav.providers') }}</p>
          <span class="overview-card-icon"><Boxes class="size-5" aria-hidden="true" /></span>
        </div>
        <p class="mt-4 text-3xl font-semibold tracking-tight tabular-nums">{{ countsVisible ? admin.providers.length : dash }}</p>
        <div class="mt-3 space-y-1.5">
          <div class="flex h-1.5 gap-0.5 overflow-hidden rounded-full bg-muted">
            <template v-if="countsVisible">
              <div class="bg-success transition-all duration-700" :style="{ width: share(providerBreakdown.ready, admin.providers.length) }" />
              <div class="bg-muted-foreground/40 transition-all duration-700" :style="{ width: share(providerBreakdown.disabled, admin.providers.length) }" />
              <div class="bg-danger transition-all duration-700" :style="{ width: share(providerBreakdown.error, admin.providers.length) }" />
            </template>
          </div>
          <p class="text-xs text-muted-foreground">
            {{ countsVisible ? t('overview.providersReady', { ready: providerBreakdown.ready, issues: providerBreakdown.error }) : dash }}
          </p>
        </div>
      </RouterLink>
    </div>

    <!-- Pipeline + attention -->
    <div class="grid items-stretch gap-4 xl:grid-cols-[minmax(0,1.6fr)_minmax(320px,1fr)]">
      <section class="studio-panel p-5 sm:p-6" aria-labelledby="overview-pipeline-title">
        <div class="mb-5 flex items-start justify-between gap-4">
          <div>
            <h2 id="overview-pipeline-title" class="text-lg font-semibold">{{ t('overview.pipeline.title') }}</h2>
            <p class="mt-1 text-sm text-muted-foreground">{{ t('overview.pipeline.hint') }}</p>
          </div>
          <RouterLink to="/providers" class="inline-flex shrink-0 items-center gap-1 text-sm font-medium text-studio-violet hover:underline">
            {{ t('nav.providers') }} <ArrowRight class="size-4" aria-hidden="true" />
          </RouterLink>
        </div>

        <ol class="grid gap-3 md:grid-cols-4 md:gap-0">
          <li v-for="(stage, index) in pipeline" :key="stage.type" class="relative flex md:items-center">
            <div
              class="relative z-10 w-full rounded-2xl border p-4 transition md:mx-2"
              :class="{
                'border-success/40 bg-success/5': stage.tone === 'ready',
                'border-warning/50 bg-warning/5': stage.tone === 'warning',
                'border-dashed border-border bg-surface': stage.tone === 'empty',
              }"
            >
              <div class="flex items-center justify-between">
                <span
                  class="flex size-9 items-center justify-center rounded-xl"
                  :class="stage.tone === 'ready' ? 'bg-studio-cyan/15 text-studio-cyan' : stage.tone === 'warning' ? 'bg-warning/15 text-warning' : 'bg-muted text-muted-foreground'"
                >
                  <component :is="stage.icon" class="size-5" aria-hidden="true" />
                </span>
                <span class="text-xs font-bold tracking-wider text-muted-foreground">{{ stage.type.toUpperCase() }}</span>
              </div>
              <p class="mt-3 text-sm font-semibold">{{ providerTypeLabel(stage.type) }}</p>
              <p class="text-xs text-muted-foreground">{{ t(stage.hint) }}</p>
              <p class="mt-3 truncate text-xs font-medium" :class="stage.lead ? 'text-foreground' : 'text-muted-foreground'" :title="stage.lead ?? undefined">
                {{ countsVisible ? (stage.lead ?? t('overview.pipeline.none')) : dash }}
              </p>
              <p class="mt-1 text-xs tabular-nums" :class="stage.tone === 'ready' ? 'text-success' : 'text-muted-foreground'">
                {{ countsVisible ? t('overview.pipeline.ready', { ready: stage.ready, total: stage.total }) : dash }}
              </p>
            </div>
            <div
              v-if="index < pipeline.length - 1"
              class="overview-flow absolute top-1/2 -right-2 z-0 hidden h-0.5 w-4 -translate-y-1/2 md:block"
              aria-hidden="true"
            />
          </li>
        </ol>
        <p class="mt-5 text-xs text-muted-foreground">{{ t('overview.telemetryNotice') }}</p>
      </section>

      <section class="studio-panel flex flex-col p-5 sm:p-6" aria-labelledby="overview-attention-title">
        <div class="mb-4 flex items-start justify-between gap-3">
          <div>
            <h2 id="overview-attention-title" class="text-lg font-semibold">{{ t('overview.attention.title') }}</h2>
            <p class="mt-1 text-sm text-muted-foreground">{{ t('overview.attention.hint') }}</p>
          </div>
          <span
            v-if="attention.length"
            class="rounded-full bg-danger/10 px-2.5 py-0.5 text-xs font-semibold tabular-nums text-danger"
          >{{ attention.length }}</span>
        </div>

        <div v-if="!attention.length" class="flex flex-1 flex-col items-center justify-center rounded-2xl bg-success/5 px-4 py-8 text-center">
          <span class="flex size-12 items-center justify-center rounded-full bg-success/15 text-success">
            <CheckCircle2 class="size-6" aria-hidden="true" />
          </span>
          <p class="mt-3 font-semibold">{{ t('overview.attention.allClear') }}</p>
          <p class="mt-1 max-w-xs text-sm text-muted-foreground">{{ t('overview.attention.allClearHint') }}</p>
        </div>

        <ul v-else class="space-y-2">
          <li v-for="item in visibleAttention" :key="item.id">
            <RouterLink :to="item.to" class="group flex items-center gap-3 rounded-xl border border-border/70 px-3 py-2.5 text-sm transition hover:bg-accent">
              <span class="flex size-8 shrink-0 items-center justify-center rounded-lg" :class="attentionTone[item.tone].class">
                <component :is="attentionTone[item.tone].icon" class="size-4" aria-hidden="true" />
              </span>
              <span class="min-w-0 flex-1 truncate" :title="item.text">{{ item.text }}</span>
              <ArrowUpRight class="size-4 shrink-0 text-muted-foreground transition group-hover:-translate-y-0.5 group-hover:translate-x-0.5" aria-hidden="true" />
            </RouterLink>
          </li>
          <li v-if="attention.length > ATTENTION_LIMIT" class="px-1 pt-1 text-xs text-muted-foreground">
            {{ t('overview.attention.more', { count: attention.length - ATTENTION_LIMIT }) }}
          </li>
        </ul>
      </section>
    </div>

    <!-- Resources + health -->
    <div class="grid items-stretch gap-4 lg:grid-cols-[minmax(0,1.4fr)_minmax(300px,1fr)]">
      <section class="studio-panel p-5 sm:p-6" aria-labelledby="overview-resources-title">
        <div class="mb-5 flex items-start justify-between gap-4">
          <div>
            <h2 id="overview-resources-title" class="text-lg font-semibold">{{ t('overview.resources.title') }}</h2>
            <p class="mt-1 text-sm text-muted-foreground">{{ t('overview.resources.hint') }}</p>
          </div>
          <span v-if="resources" class="inline-flex items-center gap-1.5 rounded-full border border-success/40 px-2.5 py-1 text-xs text-success">
            <span class="size-1.5 animate-pulse rounded-full bg-success" aria-hidden="true" /> Live
          </span>
        </div>

        <p v-if="resourcesFailed" class="rounded-xl bg-surface p-4 text-sm text-muted-foreground">{{ t('overview.resources.unavailable') }}</p>
        <template v-else>
          <div class="grid grid-cols-3 gap-2">
            <RadialGauge v-for="gauge in gauges" :key="gauge.key" :label="gauge.label" :percentage="gauge.value" :detail="gauge.detail" />
          </div>
          <div class="mt-5 rounded-2xl bg-surface p-4">
            <div class="mb-2 flex items-center justify-between text-xs text-muted-foreground">
              <span class="font-medium">{{ t('overview.resources.trend') }}</span>
              <span class="tabular-nums">{{ cpuHistory.length ? `${cpuHistory.at(-1)!.toFixed(1)}%` : dash }}</span>
            </div>
            <Sparkline :values="cpuHistory" :label="t('overview.resources.trend')" />
            <p v-if="processLine" class="mt-2 text-xs text-muted-foreground">{{ processLine }}</p>
          </div>
        </template>
      </section>

      <section class="studio-panel p-5 sm:p-6" aria-labelledby="overview-health-title">
        <h2 id="overview-health-title" class="text-lg font-semibold">{{ t('overview.health.title') }}</h2>
        <p class="mt-1 mb-4 text-sm text-muted-foreground">{{ t('overview.health.hint') }}</p>
        <dl class="divide-y divide-border/70">
          <div v-for="row in healthRows" :key="row.label" class="flex items-center gap-3 py-2.5">
            <component :is="row.icon" class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            <dt class="flex-1 text-sm">{{ t(row.label) }}</dt>
            <dd class="flex items-center gap-2 text-sm font-medium tabular-nums">
              {{ row.value }}
              <span class="size-2 rounded-full" :class="toneDot[row.tone]" aria-hidden="true" />
            </dd>
          </div>
        </dl>
      </section>
    </div>

    <!-- Agents + quick actions -->
    <div class="grid items-start gap-4 lg:grid-cols-[minmax(0,1.4fr)_minmax(300px,1fr)]">
      <section class="studio-panel p-5 sm:p-6" aria-labelledby="overview-agents-title">
        <div class="mb-4 flex items-start justify-between gap-4">
          <div>
            <h2 id="overview-agents-title" class="text-lg font-semibold">{{ t('overview.agents.title') }}</h2>
            <p class="mt-1 text-sm text-muted-foreground">{{ t('overview.agents.hint') }}</p>
          </div>
          <RouterLink to="/agents" class="inline-flex shrink-0 items-center gap-1 text-sm font-medium text-studio-violet hover:underline">
            {{ t('overview.agents.viewAll') }} <ArrowRight class="size-4" aria-hidden="true" />
          </RouterLink>
        </div>

        <div v-if="!countsVisible" class="space-y-2">
          <div v-for="index in 3" :key="index" class="card-shimmer h-14 rounded-xl bg-surface" />
        </div>
        <p v-else-if="!roster.length" class="rounded-xl bg-surface p-6 text-center text-sm text-muted-foreground">{{ t('overview.agents.empty') }}</p>
        <ul v-else class="space-y-2">
          <li v-for="agent in roster" :key="agent.id">
            <RouterLink :to="`/agents/${encodeURIComponent(agent.id)}`" class="group flex items-center gap-3 rounded-xl px-3 py-2.5 transition hover:bg-accent">
              <span
                class="flex size-10 shrink-0 items-center justify-center rounded-xl text-sm font-semibold text-white"
                :style="{ background: `linear-gradient(135deg, hsl(${agent.hue} 70% 55%), hsl(${(agent.hue + 50) % 360} 70% 45%))` }"
                aria-hidden="true"
              >{{ agent.initials }}</span>
              <div class="min-w-0 flex-1">
                <p class="truncate text-sm font-semibold">{{ agent.name }}</p>
                <p class="flex items-center gap-1 truncate text-xs text-muted-foreground">
                  <LayoutTemplate class="size-3 shrink-0" aria-hidden="true" />
                  {{ agent.template ?? t('overview.agents.noTemplate') }}
                </p>
              </div>
              <span class="inline-flex shrink-0 items-center gap-1.5 rounded-full bg-surface px-2.5 py-1 text-xs tabular-nums text-muted-foreground">
                <span class="size-1.5 rounded-full" :class="agent.enabled ? 'bg-success' : 'bg-muted-foreground/40'" aria-hidden="true" />
                {{ t('overview.agents.devices', { count: agent.devices }) }}
              </span>
            </RouterLink>
          </li>
        </ul>
      </section>

      <section class="studio-panel p-5 sm:p-6" aria-labelledby="overview-quick-title">
        <h2 id="overview-quick-title" class="text-lg font-semibold">{{ t('overview.quickActions') }}</h2>
        <p class="mt-1 mb-4 text-sm text-muted-foreground">{{ t('overview.quickHint') }}</p>
        <div class="grid grid-cols-2 gap-2">
          <RouterLink
            v-for="item in quickActions"
            :key="item.to"
            :to="item.to"
            class="group rounded-xl border border-border/70 p-3 transition hover:-translate-y-0.5 hover:border-studio-violet/40 hover:bg-accent hover:shadow-sm"
          >
            <component :is="item.icon" class="size-5 text-studio-violet transition group-hover:scale-110" aria-hidden="true" />
            <p class="mt-2 text-sm font-semibold">{{ t(item.label) }}</p>
            <p class="truncate text-xs text-muted-foreground" :title="t(item.hint)">{{ t(item.hint) }}</p>
          </RouterLink>
        </div>
      </section>
    </div>
  </section>
</template>

<style scoped>
@reference "../style.css";

.overview-hero {
  position: relative; isolation: isolate; overflow: hidden; padding: 18px 22px; border-radius: 22px; color: #fff;
  box-shadow: 0 10px 30px -12px rgb(79 83 224 / 0.6);
  background:
    radial-gradient(120% 140% at 0% 0%, rgb(255 255 255 / 0.18), transparent 55%),
    linear-gradient(125deg, #6944e9 0%, #4f53e0 45%, #0798b9 100%);
}

.overview-hero-row { position: relative; z-index: 2; display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; }
.overview-hero-copy { min-width: 0; max-width: 640px; }
.overview-eyebrow-row { display: flex; align-items: center; flex-wrap: wrap; gap: 8px 12px; }
.overview-health { display: inline-flex; align-items: center; gap: 6px; padding: 2px 8px; border: 1px solid rgb(255 255 255 / 0.1); border-radius: 99px; background: rgb(255 255 255 / 0.1); color: rgb(255 255 255 / 0.85); font-size: 10px; line-height: 1.5; }
.overview-hero h1 { margin-top: 6px; font-size: clamp(26px, 2.4vw, 32px); line-height: 1.15; font-weight: 600; letter-spacing: -0.025em; }
.overview-hero-intro { margin-top: 6px; font-size: 14px; line-height: 1.5; color: rgb(255 255 255 / 0.82); }
.overview-hero-actions { display: flex; flex-direction: column; align-items: flex-end; gap: 8px; flex-shrink: 0; }
.overview-last-checked { max-width: 320px; font-size: 10px; line-height: 1.6; color: rgb(255 255 255 / 0.65); text-align: right; }
.overview-hero-stats { position: relative; z-index: 2; display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 10px; margin-top: 16px; }
.overview-hero-stats > div { min-width: 0; padding: 10px 14px; border: 1px solid rgb(255 255 255 / 0.08); border-radius: 14px; background: rgb(255 255 255 / 0.1); backdrop-filter: blur(8px); }
.overview-hero-stats dt { font-size: 11px; letter-spacing: .08em; text-transform: uppercase; color: rgb(255 255 255 / 0.72); }
.overview-hero-stats dd { margin-top: 3px; font-size: 24px; line-height: 1.2; font-weight: 600; font-variant-numeric: tabular-nums; overflow-wrap: anywhere; }
.overview-wave-field { position: absolute; z-index: 1; inset: 0 0 0 auto; display: flex; align-items: center; gap: 4px; width: min(70%, 820px); opacity: .2; pointer-events: none; mask-image: linear-gradient(90deg, transparent, #000 70%); }
@media (max-width: 900px) {
  .overview-hero-row { flex-direction: column; gap: 16px; }
  .overview-hero-actions { flex-direction: row; align-items: center; flex-wrap: wrap; gap: 8px 14px; }
  .overview-last-checked { text-align: left; }
}
@media (max-width: 560px) {
  .overview-hero { padding: 16px; border-radius: 18px; }
  .overview-hero-stats { grid-template-columns: minmax(0, 1fr); gap: 8px; }
  .overview-hero-stats > div { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 10px 12px; }
  .overview-hero-stats dt { font-size: 10px; letter-spacing: .04em; }
  .overview-hero-stats dd { margin-top: 0; font-size: 22px; }
  .overview-wave-field { width: 100%; opacity: .12; }
}

.overview-hero-grid {
  background-image:
    linear-gradient(rgb(255 255 255 / 0.07) 1px, transparent 1px),
    linear-gradient(90deg, rgb(255 255 255 / 0.07) 1px, transparent 1px);
  background-size: 32px 32px;
  mask-image: radial-gradient(ellipse at 80% 20%, black 10%, transparent 70%);
}

.overview-wave-bar {
  transform-origin: center;
  transform: scaleY(0.25);
  animation: overview-wave 2.8s ease-in-out infinite;
}

.overview-wave-live .overview-wave-bar {
  animation-duration: 1.1s;
}

@keyframes overview-wave {
  0%, 100% { transform: scaleY(0.2); }
  50% { transform: scaleY(0.55); }
}

.overview-wave-live .overview-wave-bar {
  animation-name: overview-wave-live;
}

@keyframes overview-wave-live {
  0%, 100% { transform: scaleY(0.25); }
  50% { transform: scaleY(1); }
}

.overview-card {
  @apply block rounded-2xl border border-border/80 bg-card p-5 text-card-foreground shadow-sm transition;
}

.overview-card:hover {
  @apply -translate-y-0.5 border-studio-violet/40 shadow-md;
}

.overview-card-icon {
  @apply flex size-10 items-center justify-center rounded-xl bg-primary/10 text-studio-violet transition group-hover:scale-110;
}

.overview-flow {
  background-image: linear-gradient(90deg, var(--studio-cyan) 50%, transparent 50%);
  background-size: 8px 2px;
  animation: overview-flow 0.8s linear infinite;
}

@keyframes overview-flow {
  to { background-position: 8px 0; }
}

@media (prefers-reduced-motion: reduce) {
  .overview-wave-bar,
  .overview-flow {
    animation: none !important;
  }
}
</style>

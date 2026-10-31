<script setup lang="ts">
import {
  ArrowRight,
  Bell,
  Bot,
  ChevronRight,
  Cpu,
  Database,
  KeyRound,
  RefreshCcw,
  ShieldCheck,
  Terminal,
  Workflow,
} from '@lucide/vue'
import { storeToRefs } from 'pinia'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { RouterLink, useRouter } from 'vue-router'

import { assistantApi, type AssistantSettings } from '@/api/assistant'
import { isApiError } from '@/api/errors'
import { systemApi } from '@/api/system'
import type { SystemResources } from '@/api/types/system'
import PageHeader from '@/components/admin/PageHeader.vue'
import ResourceStatCard from '@/components/system/ResourceStatCard.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useServerStore } from '@/stores/server'

const router = useRouter()
const server = useServerStore()
const { health, status, lastError } = storeToRefs(server)
const { t, formatDateTime } = useI18n()

const resources = ref<SystemResources | null>(null)
const assistantSummary = ref<AssistantSettings | null>(null)
const loading = ref(true)
const error = ref('')
const now = ref(Date.now())
const controller = new AbortController()
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
const healthLabel = computed(() =>
  health.value === 'online'
    ? t('status.device.online')
    : health.value === 'offline'
      ? t('status.device.offline')
      : health.value === 'checking'
        ? t('system.checking')
        : t('common.unknown'),
)
const sampledAtLabel = computed(() => {
  const sampled = resources.value && Date.parse(resources.value.sampled_at)
  return sampled && Number.isFinite(sampled) ? formatDateTime(new Date(sampled)) : '—'
})

const assistantStatLabel = computed(() => {
  if (!assistantSummary.value) return 'Studio Assistant'
  const stateLabel = assistantSummary.value.enabled ? 'Enabled' : 'Disabled'
  const agentKey = assistantSummary.value.agent_key || '—'
  return `${stateLabel} · ${agentKey}`
})

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
    assistantSummary.value = await assistantApi.settings()
  } catch {
    // Non-blocking summary on System Hub
  }
}

function refresh() {
  void server.refresh(controller.signal)
  void refreshResources()
}
function visible() {
  if (!document.hidden) {
    now.value = Date.now()
    void refreshResources()
  }
}
onMounted(() => {
  refresh()
  void loadAssistantSummary()
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
  controller.abort()
  window.clearInterval(resourcesTimer)
  window.clearInterval(statusTimer)
  document.removeEventListener('visibilitychange', visible)
})
</script>

<template>
  <section class="space-y-4">
    <!-- Hero Header -->
    <PageHeader
      :eyebrow="t('system.hub.eyebrow')"
      :title="t('system.hub.title')"
      :description="t('system.hub.subtitle')"
    >
      <template #actions>
        <Button
          variant="outline"
          size="sm"
          :disabled="loading || health === 'checking'"
          @click="refresh"
        >
          <RefreshCcw :class="['size-4', loading && 'animate-spin']" aria-hidden="true" />
          {{ t('system.refresh') }}
        </Button>
        <RouterLink
          to="/system/logs"
          class="inline-flex h-8 items-center justify-center gap-1.5 whitespace-nowrap rounded-lg border border-input bg-background px-3 text-xs font-medium text-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        >
          <span>{{ t('system.hub.openDetail') }}</span>
          <ArrowRight class="size-3.5" aria-hidden="true" />
        </RouterLink>
      </template>
    </PageHeader>

    <!-- Status Strip -->
    <section
      class="studio-panel flex flex-wrap items-center justify-between gap-3.5 p-4"
      data-testid="system-status-strip"
    >
      <div class="flex items-center gap-3">
        <span
          class="grid size-9 shrink-0 place-items-center rounded-xl"
          :class="
            health === 'online'
              ? 'bg-success/12 text-success'
              : health === 'offline'
                ? 'bg-danger/12 text-danger'
                : 'bg-warning/15 text-warning-foreground dark:text-warning'
          "
        >
          <ShieldCheck class="size-5" aria-hidden="true" />
        </span>
        <div>
          <strong class="block text-sm font-semibold">
            {{ health === 'online' ? t('system.hub.operational') : healthLabel }}
          </strong>
          <div class="text-xs text-muted-foreground">
            {{ t('system.version') }} {{ status?.version ?? '—' }} · {{ t('system.uptime') }}
            {{ status ? `${status.uptime_seconds}s` : '—' }}
          </div>
        </div>
      </div>

      <div class="flex flex-wrap items-center gap-2 text-xs">
        <Badge
          :variant="
            health === 'online' ? 'success' : health === 'offline' ? 'danger' : 'secondary'
          "
        >
          {{ healthLabel }}
        </Badge>
        <Badge variant="success">
          {{ t('system.database') }} · {{ status?.database.status ?? '—' }}
        </Badge>
        <Badge variant="default">
          Provider · {{ status?.providers.loaded ?? '—' }} loaded
        </Badge>
        <Badge variant="outline">
          {{ t('system.lastUpdated') }} · {{ sampledAtLabel }}
        </Badge>
        <RouterLink
          to="/system/logs"
          class="inline-flex h-7 items-center justify-center gap-1 rounded-md border border-input bg-background px-2.5 text-xs font-medium text-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        >
          {{ t('system.hub.viewLogs') }}
        </RouterLink>
      </div>
    </section>

    <p v-if="lastError" role="status" class="text-sm text-destructive">
      {{ t('system.statusUnavailable') }}
    </p>

    <!-- Section 1: Server Health Snapshot -->
    <div class="space-y-3 pt-1">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h2 class="text-base font-semibold tracking-tight">{{ t('system.hub.healthTitle') }}</h2>
          <small class="text-xs text-muted-foreground">
            {{ t('system.hub.healthSubtitle') }}
          </small>
        </div>
        <RouterLink
          to="/system/monitor"
          data-testid="system-open-monitor"
          class="inline-flex h-8 items-center justify-center gap-1.5 rounded-lg border border-input bg-background px-3 text-xs font-medium text-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        >
          <span>{{ t('system.hub.openDetail') }}</span>
          <ArrowRight class="size-3.5" aria-hidden="true" />
        </RouterLink>
      </div>

      <div class="grid grid-cols-1 gap-3.5 sm:grid-cols-2 xl:grid-cols-4">
        <ResourceStatCard
          role="button"
          tabindex="0"
          class="cursor-pointer transition-all hover:-translate-y-0.5 hover:border-studio-violet/45 hover:shadow-md"
          :title="t('system.cpu')"
          :value="percent(current?.system.cpu_percent)"
          :detail="`${t('system.logicalCores')}: ${current?.system.logical_cores ?? '—'}`"
          :percentage="current?.system.cpu_percent"
          :state="
            resourceState || (current?.system.cpu_percent == null ? t('system.unavailable') : '')
          "
          @click="router.push({ name: 'system-monitor' })"
          @keydown.enter.prevent="router.push({ name: 'system-monitor' })"
          @keydown.space.prevent="router.push({ name: 'system-monitor' })"
        />
        <ResourceStatCard
          role="button"
          tabindex="0"
          class="cursor-pointer transition-all hover:-translate-y-0.5 hover:border-studio-violet/45 hover:shadow-md"
          :title="t('system.ram')"
          :value="`${bytes(current?.system.memory_used_bytes)} / ${bytes(current?.system.memory_total_bytes)}`"
          :percentage="memoryPercentage"
          :state="resourceState || (memoryPercentage == null ? t('system.unavailable') : '')"
          @click="router.push({ name: 'system-monitor' })"
          @keydown.enter.prevent="router.push({ name: 'system-monitor' })"
          @keydown.space.prevent="router.push({ name: 'system-monitor' })"
        />
        <ResourceStatCard
          role="button"
          tabindex="0"
          class="cursor-pointer transition-all hover:-translate-y-0.5 hover:border-studio-violet/45 hover:shadow-md"
          :title="t('system.process')"
          :value="`RSS ${bytes(current?.process.rss_bytes)}`"
          :detail="`${t('system.processCpu')}: ${percent(current?.process.cpu_percent)}`"
          :state="
            resourceState || (current?.process.rss_bytes == null ? t('system.unavailable') : '')
          "
          @click="router.push({ name: 'system-monitor' })"
          @keydown.enter.prevent="router.push({ name: 'system-monitor' })"
          @keydown.space.prevent="router.push({ name: 'system-monitor' })"
        />
        <ResourceStatCard
          role="button"
          tabindex="0"
          class="cursor-pointer transition-all hover:-translate-y-0.5 hover:border-studio-violet/45 hover:shadow-md"
          :title="t('system.rootFilesystem')"
          :value="percent(diskPercentage)"
          :detail="`${bytes(diskUsed)} / ${bytes(current?.storage.total_bytes)}`"
          :percentage="diskPercentage"
          :state="resourceState || (diskPercentage == null ? t('system.unavailable') : '')"
          @click="router.push({ name: 'system-monitor' })"
          @keydown.enter.prevent="router.push({ name: 'system-monitor' })"
          @keydown.space.prevent="router.push({ name: 'system-monitor' })"
        />
      </div>
    </div>

    <!-- Section 2: System Configuration Modules (6 Feature Cards) -->
    <div class="space-y-3 pt-1">
      <div>
        <h2 class="text-base font-semibold tracking-tight">{{ t('system.hub.configTitle') }}</h2>
        <small class="text-xs text-muted-foreground">
          {{ t('system.hub.configSubtitle') }}
        </small>
      </div>

      <div class="grid gap-3.5 md:grid-cols-2 xl:grid-cols-3">
        <!-- 1. Assistant Device -->
        <article
          role="button"
          tabindex="0"
          data-system-card="assistant"
          class="group studio-panel flex cursor-pointer flex-col justify-between overflow-hidden transition-all hover:-translate-y-0.5 hover:border-studio-violet/45 hover:shadow-md"
          @click="router.push({ name: 'system-assistant' })"
          @keydown.enter.prevent="router.push({ name: 'system-assistant' })"
          @keydown.space.prevent="router.push({ name: 'system-assistant' })"
        >
          <div class="flex items-start gap-3 p-4">
            <span
              class="grid size-9 shrink-0 place-items-center rounded-xl bg-studio-violet/12 text-studio-violet"
            >
              <Bot class="size-5" aria-hidden="true" />
            </span>
            <div class="min-w-0 flex-1">
              <div class="flex items-center justify-between gap-2">
                <h3 class="text-sm font-semibold">{{ t('system.hub.assistantTitle') }}</h3>
                <ArrowRight
                  class="size-3.5 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-studio-violet"
                  aria-hidden="true"
                />
              </div>
              <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
                {{ t('system.hub.assistantDesc') }}
              </p>
            </div>
          </div>
          <div
            class="flex items-center justify-between gap-2 border-t border-border/60 bg-surface/60 px-4 py-2.5 text-xs"
          >
            <span class="truncate text-muted-foreground">{{ assistantStatLabel }}</span>
            <RouterLink
              to="/system/assistant"
              data-testid="system-open-assistant"
              class="inline-flex shrink-0 items-center gap-1 rounded-md px-2 py-1 font-medium text-muted-foreground transition-colors group-hover:text-studio-violet hover:bg-accent hover:text-foreground"
              @click.stop
            >
              <span>{{ t('system.hub.configureView') }}</span>
              <ChevronRight class="size-3.5" aria-hidden="true" />
            </RouterLink>
          </div>
        </article>

        <!-- 2. System / Knowledge MCP -->
        <article
          role="button"
          tabindex="0"
          data-system-card="mcp"
          class="group studio-panel flex cursor-pointer flex-col justify-between overflow-hidden transition-all hover:-translate-y-0.5 hover:border-studio-violet/45 hover:shadow-md"
          @click="router.push({ name: 'system-mcp' })"
          @keydown.enter.prevent="router.push({ name: 'system-mcp' })"
          @keydown.space.prevent="router.push({ name: 'system-mcp' })"
        >
          <div class="flex items-start gap-3 p-4">
            <span
              class="grid size-9 shrink-0 place-items-center rounded-xl bg-studio-cyan/12 text-studio-cyan"
            >
              <KeyRound class="size-5" aria-hidden="true" />
            </span>
            <div class="min-w-0 flex-1">
              <div class="flex items-center justify-between gap-2">
                <h3 class="text-sm font-semibold">{{ t('system.hub.mcpTitle') }}</h3>
                <ArrowRight
                  class="size-3.5 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-studio-violet"
                  aria-hidden="true"
                />
              </div>
              <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
                {{ t('system.hub.mcpDesc') }}
              </p>
            </div>
          </div>
          <div
            class="flex items-center justify-between gap-2 border-t border-border/60 bg-surface/60 px-4 py-2.5 text-xs"
          >
            <span class="truncate text-muted-foreground">{{ t('system.hub.mcpStat') }}</span>
            <RouterLink
              to="/system/mcp"
              data-testid="system-open-mcp-settings"
              class="inline-flex shrink-0 items-center gap-1 rounded-md px-2 py-1 font-medium text-muted-foreground transition-colors group-hover:text-studio-violet hover:bg-accent hover:text-foreground"
              @click.stop
            >
              <span>{{ t('system.hub.configureView') }}</span>
              <ChevronRight class="size-3.5" aria-hidden="true" />
            </RouterLink>
          </div>
        </article>

        <!-- 3. Notifications -->
        <article
          role="button"
          tabindex="0"
          data-system-card="notifications"
          class="group studio-panel flex cursor-pointer flex-col justify-between overflow-hidden transition-all hover:-translate-y-0.5 hover:border-studio-violet/45 hover:shadow-md"
          @click="router.push({ name: 'system-notifications' })"
          @keydown.enter.prevent="router.push({ name: 'system-notifications' })"
          @keydown.space.prevent="router.push({ name: 'system-notifications' })"
        >
          <div class="flex items-start gap-3 p-4">
            <span
              class="grid size-9 shrink-0 place-items-center rounded-xl bg-success/12 text-success"
            >
              <Bell class="size-5" aria-hidden="true" />
            </span>
            <div class="min-w-0 flex-1">
              <div class="flex items-center justify-between gap-2">
                <h3 class="text-sm font-semibold">{{ t('system.hub.notificationsTitle') }}</h3>
                <ArrowRight
                  class="size-3.5 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-studio-violet"
                  aria-hidden="true"
                />
              </div>
              <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
                {{ t('system.hub.notificationsDesc') }}
              </p>
            </div>
          </div>
          <div
            class="flex items-center justify-between gap-2 border-t border-border/60 bg-surface/60 px-4 py-2.5 text-xs"
          >
            <span class="truncate text-muted-foreground">
              {{ t('system.hub.notificationsStat') }}
            </span>
            <RouterLink
              to="/system/notifications"
              data-testid="system-open-notification-settings"
              class="inline-flex shrink-0 items-center gap-1 rounded-md px-2 py-1 font-medium text-muted-foreground transition-colors group-hover:text-studio-violet hover:bg-accent hover:text-foreground"
              @click.stop
            >
              <span>{{ t('system.hub.configureView') }}</span>
              <ChevronRight class="size-3.5" aria-hidden="true" />
            </RouterLink>
          </div>
        </article>

        <!-- 4. Backup & Restore -->
        <article
          role="button"
          tabindex="0"
          data-system-card="backup"
          class="group studio-panel flex cursor-pointer flex-col justify-between overflow-hidden transition-all hover:-translate-y-0.5 hover:border-studio-violet/45 hover:shadow-md"
          @click="router.push({ name: 'system-backup' })"
          @keydown.enter.prevent="router.push({ name: 'system-backup' })"
          @keydown.space.prevent="router.push({ name: 'system-backup' })"
        >
          <div class="flex items-start gap-3 p-4">
            <span
              class="grid size-9 shrink-0 place-items-center rounded-xl bg-warning/15 text-warning-foreground dark:text-warning"
            >
              <Database class="size-5" aria-hidden="true" />
            </span>
            <div class="min-w-0 flex-1">
              <div class="flex items-center justify-between gap-2">
                <h3 class="text-sm font-semibold">{{ t('system.hub.backupTitle') }}</h3>
                <ArrowRight
                  class="size-3.5 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-studio-violet"
                  aria-hidden="true"
                />
              </div>
              <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
                {{ t('system.hub.backupDesc') }}
              </p>
            </div>
          </div>
          <div
            class="flex items-center justify-between gap-2 border-t border-border/60 bg-surface/60 px-4 py-2.5 text-xs"
          >
            <span class="truncate text-muted-foreground">{{ t('system.hub.backupStat') }}</span>
            <RouterLink
              to="/system/backup"
              data-testid="system-open-backup"
              class="inline-flex shrink-0 items-center gap-1 rounded-md px-2 py-1 font-medium text-muted-foreground transition-colors group-hover:text-studio-violet hover:bg-accent hover:text-foreground"
              @click.stop
            >
              <span>{{ t('system.hub.configureView') }}</span>
              <ChevronRight class="size-3.5" aria-hidden="true" />
            </RouterLink>
          </div>
        </article>

        <!-- 5. AI Runtime -->
        <article
          role="button"
          tabindex="0"
          data-system-card="runtime"
          class="group studio-panel flex cursor-pointer flex-col justify-between overflow-hidden transition-all hover:-translate-y-0.5 hover:border-studio-violet/45 hover:shadow-md"
          @click="router.push({ name: 'system-runtime' })"
          @keydown.enter.prevent="router.push({ name: 'system-runtime' })"
          @keydown.space.prevent="router.push({ name: 'system-runtime' })"
        >
          <div class="flex items-start gap-3 p-4">
            <span
              class="grid size-9 shrink-0 place-items-center rounded-xl bg-studio-violet/12 text-studio-violet"
            >
              <Cpu class="size-5" aria-hidden="true" />
            </span>
            <div class="min-w-0 flex-1">
              <div class="flex items-center justify-between gap-2">
                <h3 class="text-sm font-semibold">{{ t('system.hub.runtimeTitle') }}</h3>
                <ArrowRight
                  class="size-3.5 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-studio-violet"
                  aria-hidden="true"
                />
              </div>
              <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
                {{ t('system.hub.runtimeDesc') }}
              </p>
            </div>
          </div>
          <div
            class="flex items-center justify-between gap-2 border-t border-border/60 bg-surface/60 px-4 py-2.5 text-xs"
          >
            <span class="truncate text-muted-foreground">
              {{ status?.providers.loaded ?? '—' }} loaded · {{ t('system.estimatedReservation') }}:
              {{ bytes(status?.provider_runtime?.accounting.reserved_bytes) }}
            </span>
            <RouterLink
              to="/system/runtime"
              data-testid="system-open-runtime"
              class="inline-flex shrink-0 items-center gap-1 rounded-md px-2 py-1 font-medium text-muted-foreground transition-colors group-hover:text-studio-violet hover:bg-accent hover:text-foreground"
              @click.stop
            >
              <span>{{ t('system.hub.configureView') }}</span>
              <ChevronRight class="size-3.5" aria-hidden="true" />
            </RouterLink>
          </div>
        </article>

        <!-- 6. Server & Transport -->
        <article
          role="button"
          tabindex="0"
          data-system-card="transport"
          class="group studio-panel flex cursor-pointer flex-col justify-between overflow-hidden transition-all hover:-translate-y-0.5 hover:border-studio-violet/45 hover:shadow-md"
          @click="router.push({ name: 'system-transport' })"
          @keydown.enter.prevent="router.push({ name: 'system-transport' })"
          @keydown.space.prevent="router.push({ name: 'system-transport' })"
        >
          <div class="flex items-start gap-3 p-4">
            <span
              class="grid size-9 shrink-0 place-items-center rounded-xl bg-studio-violet/12 text-studio-violet"
            >
              <Workflow class="size-5" aria-hidden="true" />
            </span>
            <div class="min-w-0 flex-1">
              <div class="flex items-center justify-between gap-2">
                <h3 class="text-sm font-semibold">{{ t('system.hub.transportTitle') }}</h3>
                <ArrowRight
                  class="size-3.5 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-studio-violet"
                  aria-hidden="true"
                />
              </div>
              <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
                {{ t('system.hub.transportDesc') }}
              </p>
            </div>
          </div>
          <div
            class="flex items-center justify-between gap-2 border-t border-border/60 bg-surface/60 px-4 py-2.5 text-xs"
          >
            <span class="truncate text-muted-foreground">
              Rust / Axum · PID {{ current?.process.pid ?? '—' }}
            </span>
            <RouterLink
              to="/system/transport"
              data-testid="system-open-transport"
              class="inline-flex shrink-0 items-center gap-1 rounded-md px-2 py-1 font-medium text-muted-foreground transition-colors group-hover:text-studio-violet hover:bg-accent hover:text-foreground"
              @click.stop
            >
              <span>{{ t('system.hub.configureView') }}</span>
              <ChevronRight class="size-3.5" aria-hidden="true" />
            </RouterLink>
          </div>
        </article>
      </div>
    </div>

    <!-- Section 3: Diagnostics & Audit -->
    <div class="space-y-3 pt-1">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h2 class="text-base font-semibold tracking-tight">
            {{ t('system.hub.diagnosticsTitle') }}
          </h2>
          <small class="text-xs text-muted-foreground">
            {{ t('system.hub.diagnosticsSubtitle') }}
          </small>
        </div>
        <RouterLink
          to="/system/logs"
          data-testid="system-open-logs"
          class="inline-flex h-8 items-center justify-center gap-1.5 rounded-lg border border-input bg-background px-3 text-xs font-medium text-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        >
          <span>{{ t('system.hub.openDetail') }}</span>
          <ArrowRight class="size-3.5" aria-hidden="true" />
        </RouterLink>
      </div>

      <section
        role="button"
        tabindex="0"
        data-system-card="logs"
        class="group studio-panel flex cursor-pointer flex-wrap items-center justify-between gap-3.5 p-4 transition-all hover:-translate-y-0.5 hover:border-studio-violet/45 hover:shadow-md"
        @click="router.push({ name: 'system-logs' })"
        @keydown.enter.prevent="router.push({ name: 'system-logs' })"
        @keydown.space.prevent="router.push({ name: 'system-logs' })"
      >
        <div class="flex items-center gap-3">
          <span
            class="grid size-9 shrink-0 place-items-center rounded-xl bg-success/12 text-success"
          >
            <Terminal class="size-5" aria-hidden="true" />
          </span>
          <div>
            <strong class="block text-sm font-semibold">
              {{ t('system.hub.logsBannerTitle') }}
            </strong>
            <p class="text-xs text-muted-foreground">
              {{ t('system.hub.logsBannerDesc') }}
            </p>
          </div>
        </div>

        <RouterLink
          to="/system/logs"
          class="inline-flex h-8 items-center justify-center gap-1.5 rounded-lg border border-input bg-background px-3 text-xs font-medium text-foreground transition-colors group-hover:border-studio-violet/40 hover:bg-accent hover:text-accent-foreground"
          @click.stop
        >
          <span>{{ t('system.logs.openConsole') }}</span>
          <ArrowRight class="size-3.5" aria-hidden="true" />
        </RouterLink>
      </section>
    </div>
  </section>
</template>

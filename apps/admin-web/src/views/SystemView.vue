<script setup lang="ts">
import { Bell, RefreshCcw, ScrollText, Settings2 } from '@lucide/vue'
import { storeToRefs } from 'pinia'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { RouterLink } from 'vue-router'

import { isApiError } from '@/api/errors'
import { systemApi } from '@/api/system'
import type { SystemResources } from '@/api/types/system'
import AssistantDeviceSettings from '@/components/system/AssistantDeviceSettings.vue'
import ResourceStatCard from '@/components/system/ResourceStatCard.vue'
import PageHeader from '@/components/admin/PageHeader.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { useI18n } from '@/composables/useI18n'
import { useServerStore } from '@/stores/server'

const server = useServerStore()
const { health, status, lastError } = storeToRefs(server)
const { t, formatDateTime } = useI18n()
const resources = ref<SystemResources | null>(null)
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
const current = computed(() => !error.value && !stale.value ? resources.value : null)
const resourceState = computed(() => error.value || (stale.value ? t('system.stale') : loading.value && !resources.value ? t('common.loading') : ''))
const healthLabel = computed(() => health.value === 'online' ? t('status.device.online')
  : health.value === 'offline' ? t('status.device.offline')
    : health.value === 'checking' ? t('system.checking') : t('common.unknown'))
const sampledAtLabel = computed(() => {
  const sampled = resources.value && Date.parse(resources.value.sampled_at)
  return sampled && Number.isFinite(sampled) ? formatDateTime(new Date(sampled)) : '—'
})
function bytes(value: number | null | undefined) {
  if (value == null || !Number.isFinite(value) || value < 0) return '—'
  return value >= 1024 ** 3 ? `${(value / 1024 ** 3).toFixed(1)} GiB` : `${(value / 1024 ** 2).toFixed(1)} MiB`
}
function percent(value: number | null | undefined) {
  return value == null || !Number.isFinite(value) || value < 0 ? '—' : `${value.toFixed(1)}%`
}
function percentage(used: number | null | undefined, total: number | null | undefined) {
  return used == null || total == null || !Number.isFinite(used) || !Number.isFinite(total) || used < 0 || total <= 0
    ? null : Math.min(100, Math.max(0, used / total * 100))
}
const memoryPercentage = computed(() => percentage(current.value?.system.memory_used_bytes, current.value?.system.memory_total_bytes))
const diskUsed = computed(() => {
  const disk = current.value?.storage
  return disk?.total_bytes != null && disk.available_bytes != null && Number.isFinite(disk.total_bytes) && Number.isFinite(disk.available_bytes) && disk.total_bytes >= 0 && disk.available_bytes >= 0
    ? Math.max(0, disk.total_bytes - disk.available_bytes) : null
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
    error.value = isApiError(cause) && cause.status === 401 ? t('system.metricsUnauthorized') : t('system.unavailable')
  } finally {
    now.value = Date.now()
    inFlight = false
    loading.value = false
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
  <section class="space-y-6">
    <PageHeader :eyebrow="t('system.eyebrow')" :title="t('system.title')" :description="t('system.description')">
      <template #actions>
        <Button variant="outline" :disabled="loading || health === 'checking'" @click="refresh">
          <RefreshCcw :class="['size-4', loading && 'animate-spin']" />{{ t('system.refresh') }}
        </Button>
      </template>
    </PageHeader>
    <div class="flex flex-wrap items-center gap-3 text-sm">
      <Badge :variant="health === 'online' ? 'success' : health === 'offline' ? 'danger' : 'secondary'">{{ healthLabel }}</Badge>
      <span>{{ t('system.version') }}: {{ status?.version ?? '—' }}</span>
      <span>{{ t('system.uptime') }}: {{ status ? `${status.uptime_seconds}s` : '—' }}</span>
      <span>{{ t('system.lastUpdated') }}: {{ sampledAtLabel }}</span>
    </div>
    <p v-if="lastError" role="status" class="text-sm text-destructive">{{ t('system.statusUnavailable') }}</p>
    <div class="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
      <ResourceStatCard :title="t('system.cpu')" :value="percent(current?.system.cpu_percent)" :detail="`${t('system.logicalCores')}: ${current?.system.logical_cores ?? '—'}`" :percentage="current?.system.cpu_percent" :state="resourceState || (current?.system.cpu_percent == null ? t('system.unavailable') : '')" />
      <ResourceStatCard :title="t('system.ram')" :value="`${bytes(current?.system.memory_used_bytes)} / ${bytes(current?.system.memory_total_bytes)}`" :percentage="memoryPercentage" :state="resourceState || (memoryPercentage == null ? t('system.unavailable') : '')" />
      <ResourceStatCard :title="t('system.process')" :value="`RSS ${bytes(current?.process.rss_bytes)}`" :detail="`${t('system.processCpu')}: ${percent(current?.process.cpu_percent)}`" :state="resourceState || (current?.process.rss_bytes == null ? t('system.unavailable') : '')" />
      <ResourceStatCard :title="t('system.rootFilesystem')" :value="percent(diskPercentage)" :detail="`${bytes(diskUsed)} / ${bytes(current?.storage.total_bytes)}`" :percentage="diskPercentage" :state="resourceState || (diskPercentage == null ? t('system.unavailable') : '')" />
    </div>
    <Card>
      <CardHeader><CardTitle>{{ t('system.aiRuntime') }}</CardTitle></CardHeader>
      <CardContent class="grid gap-3 text-sm sm:grid-cols-2 xl:grid-cols-3">
        <p>{{ t('system.sessions') }}: {{ status?.sessions.active ?? '—' }}</p>
        <p>{{ t('system.loadedProviders') }}: {{ status?.providers.loaded ?? '—' }}</p>
        <p>{{ t('system.failedProviders') }}: {{ status?.providers.failed ?? '—' }}</p>
        <p>{{ t('system.staleProviders') }}: {{ status?.providers.stale ?? '—' }}</p>
        <p>{{ t('system.estimatedReservation') }}: {{ bytes(status?.provider_runtime?.accounting.reserved_bytes) }}</p>
        <p>{{ t('system.activeLeases') }}: {{ status?.provider_runtime?.accounting.active_leases ?? '—' }}</p>
      </CardContent>
    </Card>
    <AssistantDeviceSettings />
    <Card>
      <CardHeader class="flex flex-row flex-wrap items-center justify-between gap-3">
        <div class="flex items-center gap-3">
          <div class="grid size-10 shrink-0 place-items-center rounded-xl bg-studio-violet/10 text-studio-violet">
            <Bell class="size-5" />
          </div>
          <div class="space-y-0.5">
            <p class="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
              {{ t('notifications.settings.eyebrow') }}
            </p>
            <CardTitle>{{ t('notifications.settings.title') }}</CardTitle>
          </div>
        </div>
        <RouterLink
          to="/system/notifications"
          data-testid="system-open-notification-settings"
          class="inline-flex h-8 items-center justify-center gap-2 whitespace-nowrap rounded-md border border-input bg-background px-3 text-xs font-medium text-foreground transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <Settings2 class="size-4 shrink-0" />
          <span>{{ t('notifications.settings.title') }}</span>
        </RouterLink>
      </CardHeader>
      <CardContent class="text-sm text-muted-foreground">
        {{ t('notifications.settings.description') }}
      </CardContent>
    </Card>
    <Card>
      <CardHeader class="flex flex-row flex-wrap items-center justify-between gap-3">
        <div class="flex items-center gap-3">
          <div class="grid size-10 shrink-0 place-items-center rounded-xl bg-emerald-500/10 text-emerald-600 dark:text-emerald-400">
            <ScrollText class="size-5" />
          </div>
          <div class="space-y-0.5">
            <p class="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
              {{ t('system.logs.eyebrow') }}
            </p>
            <CardTitle>{{ t('system.logs.title') }}</CardTitle>
          </div>
        </div>
        <RouterLink
          to="/system/logs"
          data-testid="system-open-logs"
          class="inline-flex h-8 items-center justify-center gap-2 whitespace-nowrap rounded-md border border-input bg-background px-3 text-xs font-medium text-foreground transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <ScrollText class="size-4 shrink-0" />
          <span>{{ t('system.logs.openConsole') }}</span>
        </RouterLink>
      </CardHeader>
      <CardContent class="text-sm text-muted-foreground">
        {{ t('system.logs.description') }}
      </CardContent>
    </Card>
    <Card>
      <CardHeader><CardTitle>{{ t('system.server') }} / {{ t('system.transport') }}</CardTitle></CardHeader>
      <CardContent class="grid gap-3 break-words text-sm sm:grid-cols-2">
        <p>{{ t('system.serverRuntime') }}: Rust / Axum</p>
        <p>PID: {{ current?.process.pid ?? '—' }}</p>
        <p>{{ t('system.database') }}: {{ status?.database.status ?? '—' }}</p>
        <p>OTA: <code>/voice/ota/</code></p>
        <p>WS: <code>/voice/v1/</code></p>
        <p>Vision: <code>/mcp/vision/explain</code></p>
      </CardContent>
    </Card>
  </section>
</template>

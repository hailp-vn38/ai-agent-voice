<script setup lang="ts">
import { Activity, Cpu, ExternalLink, HardDrive, Layers, RefreshCcw } from '@lucide/vue'
import { storeToRefs } from 'pinia'
import { computed, onBeforeUnmount, onMounted } from 'vue'
import { useRouter } from 'vue-router'

import DetailHeader from '@/components/admin/DetailHeader.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useServerStore } from '@/stores/server'

const router = useRouter()
const server = useServerStore()
const { status, health } = storeToRefs(server)
const { t } = useI18n()

const controller = new AbortController()
let statusTimer: number | undefined

function bytes(value: number | null | undefined) {
  if (value == null || !Number.isFinite(value) || value < 0) return '—'
  return value >= 1024 ** 3
    ? `${(value / 1024 ** 3).toFixed(1)} GiB`
    : `${(value / 1024 ** 2).toFixed(1)} MiB`
}

const providerSlotPercent = computed(() => {
  const loaded = status.value?.providers.loaded ?? 0
  const configured = status.value?.providers.configured ?? 0
  if (!configured || configured <= 0) return loaded > 0 ? 100 : 0
  return Math.min(100, Math.max(0, Math.round((loaded / configured) * 100)))
})

const sessionPercent = computed(() => {
  const active = status.value?.sessions.active ?? 0
  return Math.min(100, Math.max(0, active * 12))
})

const leasePercent = computed(() => {
  const leases = status.value?.provider_runtime?.accounting.active_leases ?? 0
  return Math.min(100, Math.max(0, leases * 20))
})

function refresh() {
  void server.refresh(controller.signal)
}

onMounted(() => {
  refresh()
  statusTimer = window.setInterval(() => {
    if (!document.hidden) void server.refresh(controller.signal)
  }, 15_000)
})

onBeforeUnmount(() => {
  controller.abort()
  window.clearInterval(statusTimer)
})
</script>

<template>
  <section class="space-y-4" data-testid="system-runtime-page">
    <DetailHeader theme="system"
      :title="t('system.hub.runtimeTitle')"
      :back-label="t('system.hub.backOverview')"
      @back="router.push({ name: 'system' })"
    >
      <template #icon>
        <div
          class="grid size-9 shrink-0 place-items-center rounded-xl bg-studio-violet/12 text-studio-violet"
        >
          <Cpu class="size-4.5" aria-hidden="true" />
        </div>
      </template>
      <template #details>
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span class="font-semibold uppercase tracking-wider text-studio-violet">
            {{ t('system.hub.settingsEyebrow') }}
          </span>
          <span aria-hidden="true">·</span>
          <span>{{ t('system.hub.runtimeSub') }}</span>
        </div>
      </template>
      <template #actions>
        <Button
          variant="outline"
          size="sm"
          @click="router.push({ name: 'providers' })"
        >
          <ExternalLink class="size-4" aria-hidden="true" />
          {{ t('system.providers') }}
        </Button>
        <Button
          variant="outline"
          size="sm"
          :disabled="health === 'checking'"
          data-testid="runtime-refresh-btn"
          @click="refresh"
        >
          <RefreshCcw :class="['size-4', health === 'checking' && 'animate-spin']" aria-hidden="true" />
          {{ t('system.refresh') }}
        </Button>
      </template>
    </DetailHeader>

    <!-- 3 KPI Cards -->
    <div class="grid gap-3.5 md:grid-cols-3">
      <div class="studio-panel p-4">
        <div class="flex items-center justify-between text-xs text-muted-foreground">
          <span>{{ t('system.hub.providerSlots') }}</span>
          <Cpu class="size-4" aria-hidden="true" />
        </div>
        <p class="mt-2 text-xl font-bold tracking-tight tabular-nums">
          {{ status?.providers.loaded ?? '—' }} / {{ status?.providers.configured ?? '—' }}
        </p>
        <div class="mt-2.5 h-1.5 overflow-hidden rounded-full bg-muted">
          <div class="h-full rounded-full bg-success" :style="{ width: `${providerSlotPercent}%` }" />
        </div>
        <p class="mt-2 text-[11px] text-muted-foreground">
          {{ t('system.failedProviders') }}: {{ status?.providers.failed ?? 0 }} ·
          {{ t('system.staleProviders') }}: {{ status?.providers.stale ?? 0 }}
        </p>
      </div>

      <div class="studio-panel p-4">
        <div class="flex items-center justify-between text-xs text-muted-foreground">
          <span>{{ t('system.sessions') }}</span>
          <Activity class="size-4" aria-hidden="true" />
        </div>
        <p class="mt-2 text-xl font-bold tracking-tight tabular-nums">
          {{ status?.sessions.active ?? '—' }}
        </p>
        <div class="mt-2.5 h-1.5 overflow-hidden rounded-full bg-muted">
          <div class="h-full rounded-full bg-studio-cyan" :style="{ width: `${sessionPercent}%` }" />
        </div>
        <p class="mt-2 text-[11px] text-muted-foreground">
          {{ t('system.serverRuntime') }} · Rust / Axum
        </p>
      </div>

      <div class="studio-panel p-4">
        <div class="flex items-center justify-between text-xs text-muted-foreground">
          <span>{{ t('system.estimatedReservation') }}</span>
          <HardDrive class="size-4" aria-hidden="true" />
        </div>
        <p class="mt-2 text-xl font-bold tracking-tight tabular-nums">
          {{ bytes(status?.provider_runtime?.accounting.reserved_bytes) }}
        </p>
        <div class="mt-2.5 h-1.5 overflow-hidden rounded-full bg-muted">
          <div class="h-full rounded-full bg-warning" :style="{ width: `${leasePercent}%` }" />
        </div>
        <p class="mt-2 text-[11px] text-muted-foreground">
          {{ t('system.activeLeases') }}: {{ status?.provider_runtime?.accounting.active_leases ?? '—' }}
        </p>
      </div>
    </div>

    <!-- Provider Runtime Section -->
    <div class="space-y-2.5 pt-1">
      <div>
        <h2 class="text-sm font-semibold">{{ t('system.hub.providerRuntimeTitle') }}</h2>
        <p class="text-xs text-muted-foreground">{{ t('system.hub.providerRuntimeSub') }}</p>
      </div>

      <section class="studio-panel p-4">
        <div class="mb-3 flex flex-wrap items-center justify-between gap-2">
          <div class="flex items-center gap-2.5">
            <span
              class="grid size-8 shrink-0 place-items-center rounded-lg bg-studio-violet/12 text-studio-violet"
            >
              <Layers class="size-4" aria-hidden="true" />
            </span>
            <div>
              <h3 class="text-xs font-semibold">{{ t('system.loadedProviders') }}</h3>
              <p class="text-[11px] text-muted-foreground">{{ t('system.hub.runtimeSub') }}</p>
            </div>
          </div>

          <Badge :variant="(status?.providers.failed ?? 0) > 0 ? 'outline' : 'success'">
            {{ status?.providers.loaded ?? 0 }} operational
          </Badge>
        </div>

        <div class="divide-y divide-border/60 text-xs">
          <div class="flex items-center justify-between py-2.5">
            <span>{{ t('system.loadedProviders') }}</span>
            <strong class="font-mono">{{ status?.providers.loaded ?? '—' }}</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span>{{ t('system.failedProviders') }}</span>
            <strong class="font-mono">{{ status?.providers.failed ?? '—' }}</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span>{{ t('system.staleProviders') }}</span>
            <strong class="font-mono">{{ status?.providers.stale ?? '—' }}</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span>{{ t('system.sessions') }}</span>
            <strong class="font-mono">{{ status?.sessions.active ?? '—' }}</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span>{{ t('system.activeLeases') }}</span>
            <strong class="font-mono">
              {{ status?.provider_runtime?.accounting.active_leases ?? '—' }}
            </strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span>{{ t('system.estimatedReservation') }}</span>
            <strong class="font-mono">
              {{ bytes(status?.provider_runtime?.accounting.reserved_bytes) }}
            </strong>
          </div>
        </div>
      </section>
    </div>

    <!-- Notice -->
    <div
      class="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-studio-violet/25 bg-studio-violet/8 px-3.5 py-2.5 text-xs text-foreground"
    >
      <span>{{ t('system.hub.runtimeNotice') }}</span>
      <Button
        size="sm"
        variant="outline"
        type="button"
        class="h-7 px-2.5 text-xs"
        @click="router.push({ name: 'providers' })"
      >
        {{ t('system.providers') }}
        <ExternalLink class="size-3.5" aria-hidden="true" />
      </Button>
    </div>
  </section>
</template>

<script setup lang="ts">
import { Activity, RefreshCcw } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'

import DetailHeader from '@/components/admin/DetailHeader.vue'
import ResourceStatCard from '@/components/system/ResourceStatCard.vue'
import SystemFreshnessNotice from '@/components/system/SystemFreshnessNotice.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { reconnectSystemLive, useSystemLive } from '@/composables/useSystemLive'
import { useI18n } from '@/composables/useI18n'

const router = useRouter()
const { t, formatDateTime } = useI18n()
const { snapshot: liveSnapshot, cpuHistory, connection, freshness } = useSystemLive()
const resources = computed(() => liveSnapshot.value?.resources ?? null)
const visibleCpuSamples = computed(() => cpuHistory.value.slice(-14))
const current = resources
const period = ref<'15m' | '1h' | '6h'>('15m')
const resourceState = computed(() => freshness.value.resources === 'stale'
  ? t('system.stale')
  : freshness.value.resources === 'unavailable' ? t('system.unavailable') : '')

const sampledAtLabel = computed(() => {
  const sampled = resources.value && Date.parse(resources.value.sampled_at)
  return sampled && Number.isFinite(sampled) ? formatDateTime(new Date(sampled)) : '—'
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

const sparkPoints = computed(() => {
  const raw = visibleCpuSamples.value.length >= 2 ? visibleCpuSamples.value : [25, 35]
  const stepX = 400 / (raw.length - 1)
  return raw.map((val, idx) => {
    const clamped = Math.min(100, Math.max(0, val))
    const x = Math.round(idx * stepX)
    const y = Math.round(50 - (clamped / 100) * 42)
    return { x, y }
  })
})

const sparkLinePath = computed(() =>
  sparkPoints.value.map((pt, i) => `${i === 0 ? 'M' : 'L'}${pt.x} ${pt.y}`).join(''),
)

const sparkAreaPath = computed(() => {
  const pts = sparkPoints.value
  if (!pts.length) return ''
  return `M0 55L${pts.map(pt => `${pt.x} ${pt.y}`).join('L')}L400 55Z`
})

</script>

<template>
  <section class="space-y-4" data-testid="system-monitor-page">
    <DetailHeader theme="system"
      :title="t('system.hub.monitorTitle')"
      :back-label="t('system.hub.backOverview')"
      @back="router.push({ name: 'system' })"
    >
      <template #icon>
        <div
          class="grid size-9 shrink-0 place-items-center rounded-xl bg-studio-cyan/12 text-studio-cyan"
        >
          <Activity class="size-4.5" aria-hidden="true" />
        </div>
      </template>
      <template #details>
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span class="font-semibold uppercase tracking-wider text-studio-violet">
            {{ t('system.hub.settingsEyebrow') }}
          </span>
          <span aria-hidden="true">·</span>
          <span>{{ t('system.hub.monitorSub') }}</span>
        </div>
      </template>
      <template #actions>
        <Button
          v-if="connection === 'reconnecting'"
          variant="outline"
          size="sm"
          data-testid="monitor-refresh-btn"
          @click="reconnectSystemLive"
        >
          <RefreshCcw class="size-4 animate-spin" aria-hidden="true" />
          {{ t('system.refresh') }}
        </Button>
      </template>
    </DetailHeader>

    <SystemFreshnessNotice :freshness="freshness.resources" />

    <!-- 4 Resource KPI Cards -->
    <div class="grid grid-cols-1 gap-3.5 sm:grid-cols-2 xl:grid-cols-4">
      <ResourceStatCard
        :title="t('system.cpu')"
        :value="percent(current?.system.cpu_percent)"
        :detail="`${t('system.logicalCores')}: ${current?.system.logical_cores ?? '—'}`"
        :percentage="current?.system.cpu_percent"
        :state="resourceState || (current?.system.cpu_percent == null ? t('system.unavailable') : '')"
      />
      <ResourceStatCard
        :title="t('system.ram')"
        :value="`${bytes(current?.system.memory_used_bytes)} / ${bytes(current?.system.memory_total_bytes)}`"
        :percentage="memoryPercentage"
        :state="resourceState || (memoryPercentage == null ? t('system.unavailable') : '')"
      />
      <ResourceStatCard
        :title="t('system.process')"
        :value="`RSS ${bytes(current?.process.rss_bytes)}`"
        :detail="`${t('system.processCpu')}: ${percent(current?.process.cpu_percent)}`"
        :state="resourceState || (current?.process.rss_bytes == null ? t('system.unavailable') : '')"
      />
      <ResourceStatCard
        :title="t('system.rootFilesystem')"
        :value="percent(diskPercentage)"
        :detail="`${bytes(diskUsed)} / ${bytes(current?.storage.total_bytes)}`"
        :percentage="diskPercentage"
        :state="resourceState || (diskPercentage == null ? t('system.unavailable') : '')"
      />
    </div>

    <!-- 2-Column Detail: CPU Trend + Host Details -->
    <div class="grid gap-4 lg:grid-cols-2">
      <!-- Left: CPU Utilization Sparkline -->
      <section class="studio-panel flex flex-col justify-between p-4">
        <div class="flex items-center justify-between gap-2">
          <h2 class="text-sm font-semibold">{{ t('system.hub.cpuTrendTitle') }}</h2>
          <Badge variant="outline">{{ period }}</Badge>
        </div>

        <svg
          class="my-4 h-32 w-full overflow-visible"
          viewBox="0 0 400 55"
          preserveAspectRatio="none"
          aria-hidden="true"
        >
          <path :d="sparkAreaPath" class="fill-studio-violet/15 stroke-0" />
          <path
            :d="sparkLinePath"
            class="fill-none stroke-studio-violet stroke-2"
            stroke-linejoin="round"
            stroke-linecap="round"
          />
        </svg>

        <div class="flex flex-wrap items-center justify-between gap-2">
          <span class="text-xs text-muted-foreground">{{ t('system.hub.trendSubtitle') }}</span>
          <div class="inline-flex rounded-lg border border-border bg-surface/80 p-0.5 text-xs">
            <button
              v-for="p in (['15m', '1h', '6h'] as const)"
              :key="p"
              type="button"
              :data-period="p"
              class="cursor-pointer rounded-md px-2.5 py-1 font-medium transition-colors"
              :class="
                period === p
                  ? 'bg-card text-studio-violet shadow-xs'
                  : 'text-muted-foreground hover:text-foreground'
              "
              @click="period = p"
            >
              {{ p }}
            </button>
          </div>
        </div>
      </section>

      <!-- Right: Host Details -->
      <section class="studio-panel p-4">
        <h2 class="mb-3 text-sm font-semibold">{{ t('system.hub.hostDetailsTitle') }}</h2>

        <div class="divide-y divide-border/60 text-xs">
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">{{ t('system.logicalCores') }}</span>
            <strong class="font-mono">{{ current?.system.logical_cores ?? '—' }}</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">{{ t('system.hub.processId') }}</span>
            <strong class="font-mono">{{ current?.process.pid ?? '—' }}</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">{{ t('system.processCpu') }}</span>
            <strong class="font-mono">{{ percent(current?.process.cpu_percent) }}</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">{{ t('system.ram') }}</span>
            <strong class="font-mono">
              {{ bytes(current?.system.memory_used_bytes) }} / {{ bytes(current?.system.memory_total_bytes) }}
            </strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">{{ t('system.hub.diskMount') }}</span>
            <strong class="font-mono">
              / ({{ bytes(diskUsed) }} / {{ bytes(current?.storage.total_bytes) }})
            </strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">{{ t('system.hub.samplingInterval') }}</span>
            <strong>
              {{
                t('system.hub.everySeconds', {
                  seconds: Math.round((resources?.sample_interval_ms ?? 5000) / 1000),
                })
              }}
            </strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">{{ t('system.hub.staleThreshold') }}</span>
            <strong>{{ t('system.hub.secondsValue', { seconds: 15 }) }}</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">{{ t('system.lastUpdated') }}</span>
            <strong class="font-mono">{{ sampledAtLabel }}</strong>
          </div>
        </div>
      </section>
    </div>
  </section>
</template>

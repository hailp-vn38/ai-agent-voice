<script setup lang="ts">
import { Bot, Boxes, LayoutTemplate, MonitorSmartphone, ArrowUpRight, RefreshCw } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted } from 'vue'
import { RouterLink } from 'vue-router'

import PageHeader from '@/components/admin/PageHeader.vue'
import StudioStatCard from '@/components/studio/StudioStatCard.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useAdminStore } from '@/stores/admin'
import { useAuthStore } from '@/stores/auth'
import { useServerStore } from '@/stores/server'

const { t } = useI18n()
const admin = useAdminStore()
const auth = useAuthStore()
const countsVisible = computed(() => Boolean(auth.adminToken) && !admin.loading && !admin.error)
const server = useServerStore()
const aborter = new AbortController()

async function refresh() {
  await Promise.all([admin.refreshAll(), server.refresh(aborter.signal)])
}

onMounted(() => void server.refresh(aborter.signal))
onBeforeUnmount(() => aborter.abort())
</script>

<template>
  <section class="space-y-7">
    <PageHeader :eyebrow="t('overview.eyebrow')" :title="t('overview.title')" :description="t('overview.description')">
      <template #actions>
        <Button variant="outline" :disabled="admin.loading || !auth.adminToken" @click="refresh()">
          <RefreshCw class="size-4" />
          {{ t('common.refresh') }}
        </Button>
      </template>
    </PageHeader>

    <div class="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
      <StudioStatCard :label="t('nav.agents')" :value="countsVisible ? admin.agents.length : '—'" :hint="t('overview.registered')" :icon="Bot" />
      <StudioStatCard :label="t('nav.templates')" :value="countsVisible ? admin.templates.length : '—'" :hint="t('overview.configurations')" :icon="LayoutTemplate" />
      <StudioStatCard :label="t('nav.devices')" :value="countsVisible ? admin.devices.length : '—'" :hint="t('overview.devicesHint')" :icon="MonitorSmartphone" />
      <StudioStatCard :label="t('nav.providers')" :value="countsVisible ? admin.providers.length : '—'" :hint="t('overview.catalog')" :icon="Boxes" />
    </div>

    <div class="grid items-start gap-4 lg:grid-cols-[minmax(0,1.35fr)_minmax(300px,0.65fr)]">
      <section class="studio-panel p-5 sm:p-6" aria-labelledby="overview-runtime-title">
        <div class="mb-5 flex items-start justify-between gap-4">
          <div>
            <h2 id="overview-runtime-title" class="text-lg font-semibold">{{ t('overview.runtime') }}</h2>
            <p class="mt-1 text-sm text-muted-foreground">{{ t('overview.runtimeHint') }}</p>
          </div>
          <span class="rounded-full border px-2.5 py-1 text-xs" :class="server.readiness === 'online' ? 'border-success/40 text-success' : 'border-border text-muted-foreground'">
            {{ server.readiness === 'online' ? t('overview.ready') : server.readiness === 'offline' ? t('overview.notReady') : t('common.unknown') }}
          </span>
        </div>
        <dl class="grid gap-3 sm:grid-cols-2">
          <div class="rounded-xl bg-surface p-4">
            <dt class="text-xs text-muted-foreground">{{ t('overview.activeSessions') }}</dt>
            <dd class="mt-2 text-2xl font-semibold tabular-nums">{{ server.status?.sessions.active ?? '—' }}</dd>
          </div>
          <div class="rounded-xl bg-surface p-4">
            <dt class="text-xs text-muted-foreground">{{ t('overview.loadedProviders') }}</dt>
            <dd class="mt-2 text-2xl font-semibold tabular-nums">{{ server.status?.providers.loaded ?? '—' }}</dd>
          </div>
          <div class="rounded-xl bg-surface p-4">
            <dt class="text-xs text-muted-foreground">{{ t('overview.failedProviders') }}</dt>
            <dd class="mt-2 text-2xl font-semibold tabular-nums">{{ server.status?.providers.failed ?? '—' }}</dd>
          </div>
          <div class="rounded-xl bg-surface p-4">
            <dt class="text-xs text-muted-foreground">{{ t('overview.database') }}</dt>
            <dd class="mt-2 text-sm font-semibold">{{ server.status?.database.status ?? t('common.unknown') }}</dd>
          </div>
        </dl>
        <p class="mt-4 text-xs text-muted-foreground">{{ t('overview.telemetryNotice') }}</p>
      </section>

      <section class="studio-panel p-5 sm:p-6" aria-labelledby="overview-quick-title">
        <h2 id="overview-quick-title" class="text-lg font-semibold">{{ t('overview.quickActions') }}</h2>
        <p class="mt-1 mb-4 text-sm text-muted-foreground">{{ t('overview.quickHint') }}</p>
        <div class="space-y-2">
          <RouterLink v-for="item in [
            { to: '/agents', label: t('nav.agents') },
            { to: '/templates', label: t('nav.templates') },
            { to: '/speakers', label: t('nav.speakers') },
            { to: '/devices', label: t('nav.devices') },
          ]" :key="item.to" :to="item.to" class="flex items-center justify-between rounded-lg border border-border/70 px-4 py-3 text-sm transition-colors hover:bg-accent">
            {{ item.label }}
            <ArrowUpRight class="size-4 text-muted-foreground" aria-hidden="true" />
          </RouterLink>
        </div>
      </section>
    </div>
  </section>
</template>

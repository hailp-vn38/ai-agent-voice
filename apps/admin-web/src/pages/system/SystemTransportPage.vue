<script setup lang="ts">
import { RefreshCcw, Workflow } from '@lucide/vue'
import { storeToRefs } from 'pinia'
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { systemApi } from '@/api/system'
import type { SystemResources } from '@/api/types/system'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useServerStore } from '@/stores/server'

const router = useRouter()
const server = useServerStore()
const { status, health } = storeToRefs(server)
const { t } = useI18n()

const resources = ref<SystemResources | null>(null)
const loading = ref(false)
const controller = new AbortController()

async function refresh() {
  loading.value = true
  try {
    void server.refresh(controller.signal)
    resources.value = await systemApi.resources(controller.signal)
  } catch {
    // Keep existing snapshot if unavailable.
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  void refresh()
})

onBeforeUnmount(() => {
  controller.abort()
})
</script>

<template>
  <section class="space-y-4" data-testid="system-transport-page">
    <DetailHeader
      :title="t('system.hub.transportTitle')"
      :back-label="t('system.hub.backOverview')"
      @back="router.push({ name: 'system' })"
    >
      <template #icon>
        <div
          class="grid size-9 shrink-0 place-items-center rounded-xl bg-studio-violet/12 text-studio-violet"
        >
          <Workflow class="size-4.5" aria-hidden="true" />
        </div>
      </template>
      <template #details>
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span class="font-semibold uppercase tracking-wider text-studio-violet">
            {{ t('system.hub.settingsEyebrow') }}
          </span>
          <span aria-hidden="true">·</span>
          <span>{{ t('system.hub.transportSub') }}</span>
        </div>
      </template>
      <template #actions>
        <Button
          variant="outline"
          size="sm"
          :disabled="loading || health === 'checking'"
          data-testid="transport-refresh-btn"
          @click="refresh"
        >
          <RefreshCcw
            :class="['size-4', (loading || health === 'checking') && 'animate-spin']"
            aria-hidden="true"
          />
          {{ t('system.refresh') }}
        </Button>
      </template>
    </DetailHeader>

    <div class="grid gap-4 lg:grid-cols-2">
      <!-- Server & Process -->
      <section class="studio-panel p-4">
        <div class="mb-3">
          <h2 class="text-sm font-semibold">{{ t('system.hub.serverProcessTitle') }}</h2>
          <p class="mt-0.5 text-xs text-muted-foreground">
            {{ t('system.hub.serverProcessSub') }}
          </p>
        </div>

        <div class="divide-y divide-border/60 text-xs">
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">{{ t('system.serverRuntime') }}</span>
            <strong class="font-mono">Rust / Axum</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">{{ t('system.version') }}</span>
            <strong class="font-mono">{{ status?.version ?? '—' }}</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">PID</span>
            <strong class="font-mono">{{ resources?.process.pid ?? '—' }}</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">{{ t('system.uptime') }}</span>
            <strong class="font-mono">{{ status ? `${status.uptime_seconds}s` : '—' }}</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">{{ t('system.database') }}</span>
            <strong class="font-mono">{{ status?.database.status ?? '—' }}</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">HTTP health</span>
            <strong class="font-mono">/health</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">HTTP readiness</span>
            <strong class="font-mono">/ready</strong>
          </div>
        </div>
      </section>

      <!-- Transport & API Endpoints -->
      <section class="studio-panel p-4">
        <div class="mb-3">
          <h2 class="text-sm font-semibold">{{ t('system.hub.endpointsTitle') }}</h2>
          <p class="mt-0.5 text-xs text-muted-foreground">
            {{ t('system.hub.endpointsSub') }}
          </p>
        </div>

        <div class="divide-y divide-border/60 text-xs">
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">Voice WebSocket</span>
            <strong class="font-mono">/voice/v1/</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">OTA</span>
            <strong class="font-mono">/voice/ota/</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">Vision MCP</span>
            <strong class="font-mono">/mcp/vision/explain</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">Knowledge MCP</span>
            <strong class="font-mono">/mcp/knowledge</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">Admin Resources</span>
            <strong class="font-mono">/api/admin/system/resources</strong>
          </div>
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">Admin Logs</span>
            <strong class="font-mono">/api/admin/system/logs</strong>
          </div>
        </div>
      </section>
    </div>
  </section>
</template>

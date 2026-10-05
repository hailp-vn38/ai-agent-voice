<script setup lang="ts">
import { Activity, Database, RefreshCcw, Server, Wifi } from '@lucide/vue'
import { storeToRefs } from 'pinia'
import { computed, onBeforeUnmount, onMounted } from 'vue'

import PageHeader from '@/components/admin/PageHeader.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { useI18n } from '@/composables/useI18n'
import { useAdminStore } from '@/stores/admin'
import { useServerStore } from '@/stores/server'

const server = useServerStore()
const admin = useAdminStore()
const { health, checkedAt, lastError } = storeToRefs(server)
const { t, formatDateTime } = useI18n()

const controller = new AbortController()
let intervalId: number | undefined

const healthLabel = computed(() =>
  health.value === 'online'
    ? t('status.device.online')
    : health.value === 'offline'
      ? t('status.device.offline')
      : health.value === 'checking'
        ? t('system.checking')
        : t('common.unknown'),
)
const checkedAtLabel = computed(() =>
  checkedAt.value ? formatDateTime(checkedAt.value) : t('system.notChecked'),
)

onMounted(() => {
  void server.checkHealth(controller.signal)
  intervalId = window.setInterval(() => void server.checkHealth(controller.signal), 30_000)
})

onBeforeUnmount(() => {
  controller.abort()
  if (intervalId !== undefined) window.clearInterval(intervalId)
})
</script>

<template>
  <section class="space-y-6">
    <PageHeader
      :eyebrow="t('system.eyebrow')"
      :title="t('system.title')"
      :description="t('system.description')"
    >
      <template #actions>
        <Button variant="outline" :disabled="health === 'checking'" @click="server.checkHealth()">
          <RefreshCcw :class="['size-4', health === 'checking' && 'animate-spin']" />
          {{ t('system.refresh') }}
        </Button>
      </template>
    </PageHeader>

    <div class="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
      <Card>
        <CardHeader class="pb-3">
          <div class="flex items-center justify-between">
            <CardTitle class="text-sm">{{ t('system.serverStatus') }}</CardTitle>
            <Activity class="size-4 text-muted-foreground" />
          </div>
          <CardDescription>GET /health</CardDescription>
        </CardHeader>
        <CardContent>
          <Badge :variant="health === 'online' ? 'success' : health === 'offline' ? 'danger' : 'secondary'">
            {{ healthLabel }}
          </Badge>
          <p class="mt-2 text-xs text-muted-foreground">{{ checkedAtLabel }}</p>
          <p v-if="lastError" class="mt-2 break-all text-xs text-red-600">{{ lastError }}</p>
        </CardContent>
      </Card>

      <Card>
        <CardHeader class="pb-3">
          <div class="flex items-center justify-between">
            <CardTitle class="text-sm">{{ t('system.server') }}</CardTitle>
            <Server class="size-4 text-muted-foreground" />
          </div>
          <CardDescription>voice-agent-server</CardDescription>
        </CardHeader>
        <CardContent class="space-y-2 text-sm">
          <div class="flex justify-between">
            <span class="text-muted-foreground">{{ t('system.serverRuntime') }}</span>
            <span>Rust / Axum</span>
          </div>
          <div class="flex justify-between">
            <span class="text-muted-foreground">{{ t('system.serverHealth') }}</span>
            <code>/health</code>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader class="pb-3">
          <div class="flex items-center justify-between">
            <CardTitle class="text-sm">{{ t('system.managementData') }}</CardTitle>
            <Database class="size-4 text-muted-foreground" />
          </div>
          <CardDescription>{{ t('system.managementDataHint') }}</CardDescription>
        </CardHeader>
        <CardContent class="space-y-2 text-sm">
          <div class="flex justify-between">
            <span class="text-muted-foreground">{{ t('system.agents') }}</span>
            <strong>{{ admin.agents.length }}</strong>
          </div>
          <div class="flex justify-between">
            <span class="text-muted-foreground">{{ t('system.templates') }}</span>
            <strong>{{ admin.templates.length }}</strong>
          </div>
          <div class="flex justify-between">
            <span class="text-muted-foreground">{{ t('system.providers') }}</span>
            <strong>{{ admin.providers.length }}</strong>
          </div>
          <div class="flex justify-between">
            <span class="text-muted-foreground">{{ t('system.devices') }}</span>
            <strong>{{ admin.devices.length }}</strong>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader class="pb-3">
          <div class="flex items-center justify-between">
            <CardTitle class="text-sm">{{ t('system.transport') }}</CardTitle>
            <Wifi class="size-4 text-muted-foreground" />
          </div>
          <CardDescription>{{ t('system.transportHint') }}</CardDescription>
        </CardHeader>
        <CardContent class="space-y-2 text-sm">
          <div class="flex justify-between"><span class="text-muted-foreground">OTA</span><code>/voice/ota/</code></div>
          <div class="flex justify-between"><span class="text-muted-foreground">WS</span><code>/voice/v1/</code></div>
          <div class="flex justify-between"><span class="text-muted-foreground">Vision</span><code>/mcp/vision/explain</code></div>
        </CardContent>
      </Card>
    </div>

    <Card>
      <CardHeader>
        <CardTitle>{{ t('system.apiBoundary') }}</CardTitle>
        <CardDescription>{{ t('system.apiBoundaryDescription') }}</CardDescription>
      </CardHeader>
      <CardContent class="flex flex-wrap items-center gap-3">
        <Badge variant="outline">{{ t('system.backendPending') }}</Badge>
        <Button size="sm" variant="outline" @click="admin.refreshAll()">
          {{ t('system.reset') }}
        </Button>
      </CardContent>
    </Card>
  </section>
</template>

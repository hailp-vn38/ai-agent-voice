<script setup lang="ts">
import { Activity, Check, Copy, Cpu, Network, Radio, RefreshCcw, Server, ShieldCheck } from '@lucide/vue'
import { storeToRefs } from 'pinia'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'

import { systemApi } from '@/api/system'
import type { SystemResources } from '@/api/types/system'
import SystemPageHeader from '@/components/system/SystemPageHeader.vue'
import { useI18n } from '@/composables/useI18n'
import { useServerStore } from '@/stores/server'

const server = useServerStore()
const { status, health, readiness, checkedAt, lastError } = storeToRefs(server)
const { t, formatDateTime } = useI18n()
const resources = ref<SystemResources | null>(null)
const loading = ref(false)
const resourceError = ref(false)
const copiedPath = ref('')
const copyError = ref(false)
const controller = new AbortController()

const endpointGroups = [
  { title: 'system.detail.voiceRoutes', description: 'system.detail.voiceRouteDescription', icon: Radio,
    endpoints: [{ name: 'Voice WebSocket', protocol: 'WS', path: '/voice/v1/' }, { name: 'OTA', protocol: 'HTTP', path: '/voice/ota/' }] },
  { title: 'system.detail.mcpRoutes', description: 'system.detail.mcpRouteDescription', icon: Network,
    endpoints: [{ name: 'Vision MCP', protocol: 'HTTP', path: '/mcp/vision/explain' }, { name: 'Knowledge MCP', protocol: 'HTTP', path: '/mcp/knowledge' }] },
  { title: 'system.detail.adminRoutes', description: 'system.detail.adminRouteDescription', icon: ShieldCheck,
    endpoints: [{ name: 'Admin Resources', protocol: 'GET', path: '/api/admin/system/resources' }, { name: 'Admin Logs', protocol: 'GET', path: '/api/admin/system/logs' }] },
] as const
const sampledAt = computed(() => {
  const timestamp = resources.value && Date.parse(resources.value.sampled_at)
  return timestamp && Number.isFinite(timestamp) ? formatDateTime(new Date(timestamp)) : '—'
})
function probeLabel(value: string) {
  return value === 'online' ? t('status.device.online') : value === 'offline' ? t('status.device.offline') : t('system.checking')
}
async function refresh() {
  if (loading.value || controller.signal.aborted) return
  loading.value = true
  const statusRequest = server.refresh(controller.signal)
  try {
    const snapshot = await systemApi.resources(controller.signal)
    if (!controller.signal.aborted) {
      resources.value = snapshot
      resourceError.value = false
    }
  } catch {
    if (!controller.signal.aborted) {
      resources.value = null
      resourceError.value = true
    }
  } finally {
    await statusRequest
    loading.value = false
  }
}
async function copyPath(path: string) {
  copyError.value = false
  copiedPath.value = ''
  try {
    await navigator.clipboard.writeText(path)
    copiedPath.value = path
  } catch {
    copyError.value = true
  }
}
onMounted(() => { void refresh() })
onBeforeUnmount(() => { controller.abort() })
</script>

<template>
  <section class="system-detail transport-page" data-testid="system-transport-page">
    <SystemPageHeader number="04" :eyebrow="t('system.detail.transportEyebrow')"
      :title="t('system.detail.transportTitle')" :description="t('system.detail.transportDescription')">
      <template #actions>
        <button type="button" class="sys-button" :disabled="loading" data-testid="transport-refresh-btn" @click="refresh">
          <RefreshCcw :class="['size-3.5', loading && 'animate-spin motion-reduce:animate-none']" aria-hidden="true" />{{ t('system.refresh') }}
        </button>
      </template>
    </SystemPageHeader>

    <div class="transport-layout">
      <section class="sys-panel transport-engine" aria-labelledby="transport-engine-heading">
        <header class="sys-panel-header"><span class="sys-section-number">01</span>
          <div><h2 id="transport-engine-heading">{{ t('system.detail.engine') }}</h2><p>{{ t('system.detail.engineDescription') }}</p></div>
        </header>
        <div class="transport-server-art" aria-hidden="true"><span class="sys-kicker">VELA / SERVER</span>
          <div class="transport-server-symbol"><span /><Server class="size-12" /></div>
          <span class="transport-runtime">Rust / Axum</span>
        </div>
        <div class="transport-probes">
          <div :data-probe="health"><Activity class="size-4" aria-hidden="true" /><span>{{ t('system.deck.health') }}</span><strong>{{ probeLabel(health) }}</strong><code>/health</code></div>
          <div :data-probe="readiness"><Check class="size-4" aria-hidden="true" /><span>{{ t('system.deck.readiness') }}</span><strong>{{ probeLabel(readiness) }}</strong><code>/ready</code></div>
        </div>
        <p v-if="lastError" role="status" class="sys-alert transport-error">{{ t('system.statusUnavailable') }}</p>
        <p v-if="resourceError" role="status" class="sys-alert transport-error">{{ t('system.detail.processUnavailable') }}</p>
        <dl class="transport-facts">
          <div><dt>{{ t('system.version') }}</dt><dd>{{ status?.version ?? '—' }}</dd></div>
          <div><dt>{{ t('system.hub.processId') }}</dt><dd>{{ resources?.process.pid ?? '—' }}</dd></div>
          <div><dt>{{ t('system.uptime') }}</dt><dd>{{ status ? `${status.uptime_seconds}s` : '—' }}</dd></div>
          <div><dt>{{ t('system.database') }}</dt><dd>{{ status ? status.database.enabled ? status.database.status : t('system.deck.disabled') : '—' }}</dd></div>
        </dl>
        <footer class="transport-sampled"><Cpu class="size-3.5" aria-hidden="true" />{{ t('system.detail.sampleTime', { time: sampledAt }) }}</footer>
      </section>

      <section class="transport-directory" aria-labelledby="transport-directory-heading">
        <header class="transport-directory-heading"><span class="sys-section-number">02</span>
          <div><h2 id="transport-directory-heading">{{ t('system.detail.routeBook') }}</h2><p>{{ t('system.detail.routeBookDescription') }}</p></div>
        </header>
        <p v-if="copyError" role="status" class="sys-alert">{{ t('system.detail.copyFailed') }}</p>
        <p v-if="copiedPath" role="status" class="sr-only">{{ t('system.detail.pathCopied') }}: {{ copiedPath }}</p>
        <section v-for="group in endpointGroups" :key="group.title" class="transport-route-group">
          <header><component :is="group.icon" class="size-4" aria-hidden="true" /><div><h3>{{ t(group.title) }}</h3><p>{{ t(group.description) }}</p></div></header>
          <div v-for="endpoint in group.endpoints" :key="endpoint.path" class="transport-route">
            <span class="transport-protocol">{{ endpoint.protocol }}</span>
            <div><span>{{ endpoint.name }}</span><code>{{ endpoint.path }}</code></div>
            <button type="button" :aria-label="`${t('system.detail.copyPath')}: ${endpoint.path}`" :title="t('system.detail.copyPath')" @click="copyPath(endpoint.path)">
              <component :is="copiedPath === endpoint.path ? Check : Copy" class="size-3.5" aria-hidden="true" />
            </button>
          </div>
        </section>
        <p class="transport-last-check">{{ t('system.detail.lastCheck') }} · {{ checkedAt ? formatDateTime(checkedAt) : t('system.detail.notChecked') }}</p>
      </section>
    </div>
  </section>
</template>

<style src="./system-transport.css"></style>

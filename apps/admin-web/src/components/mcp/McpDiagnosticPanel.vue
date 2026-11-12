<script setup lang="ts">
import { LoaderCircle, PlugZap, ShieldCheck, Timer, Wrench } from '@lucide/vue'
import { onBeforeUnmount, ref, watch } from 'vue'
import { mcpApi } from '@/api/mcp'
import { formatApiError } from '@/api/errors'
import type { McpDiscoveryResult, McpProbeConfig, McpProbeResult } from '@/api/types/mcp'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import McpToolCatalog from './McpToolCatalog.vue'
const props = defineProps<{ savedKey?: string; revision?: number; draft?: McpProbeConfig }>()
const { t, formatDateTime } = useI18n()
const connection = ref<McpProbeResult>()
const discovery = ref<McpDiscoveryResult>()
const busy = ref<'connection' | 'discover'>()
const error = ref('')
const failed = ref<'connection' | 'discover'>()
const testedAt = ref<Date>()
let controller: AbortController | undefined
function clear() {
  controller?.abort(); controller = undefined
  connection.value = undefined; discovery.value = undefined; error.value = ''; busy.value = undefined; failed.value = undefined; testedAt.value = undefined
}
watch(() => [props.savedKey, props.revision, props.draft], clear, { deep: true })
onBeforeUnmount(clear)
async function probe(operation: 'connection' | 'discover') {
  clear()
  const pending = new AbortController()
  controller = pending; busy.value = operation
  try {
    if (operation === 'connection') {
      const response = props.draft ? await mcpApi.testDraftConnection(props.draft, pending.signal) : await mcpApi.testSavedConnection(props.savedKey!, pending.signal)
      if (pending.signal.aborted) return
      connection.value = response
    } else {
      const response = props.draft ? await mcpApi.discoverDraftTools(props.draft, pending.signal) : await mcpApi.discoverSavedTools(props.savedKey!, pending.signal)
      if (pending.signal.aborted) return
      if (!response.complete) throw new Error('mcp_incomplete_catalog')
      discovery.value = response; connection.value = response
    }
    testedAt.value = new Date()
  } catch (cause) { if (!pending.signal.aborted) { error.value = formatApiError(cause); failed.value = operation } }
  finally { if (controller === pending) busy.value = undefined }
}
</script>
<template>
  <section class="space-y-4" aria-live="polite">
    <div class="grid gap-4 md:grid-cols-2">
      <div class="mcp-card" :data-state="busy === 'connection' ? 'busy' : connection ? 'ok' : failed === 'connection' ? 'fail' : 'idle'">
        <div class="mcp-card-head">
          <span class="mcp-card-icon" aria-hidden="true"><LoaderCircle v-if="busy === 'connection'" :size="20" class="animate-spin" /><PlugZap v-else :size="20" /></span>
          <div class="min-w-0 flex-1">
            <h2>{{ t('diagnostics.connection') }}</h2>
            <p class="mcp-card-status"><i aria-hidden="true" />{{ busy === 'connection' ? t('diagnostics.connecting') : connection ? t('diagnostics.connected') : failed === 'connection' ? t('diagnostics.failed') : t('diagnostics.notTested') }}</p>
          </div>
        </div>
        <p v-if="connection" class="mcp-card-meta"><Timer :size="13" aria-hidden="true" />{{ connection.elapsed_ms }} ms · {{ testedAt ? formatDateTime(testedAt) : '' }}</p>
        <Button type="button" variant="outline" class="mcp-card-action" data-test-connection :disabled="Boolean(busy)" @click="probe('connection')">{{ t('diagnostics.connect') }}</Button>
      </div>
      <div class="mcp-card" :data-state="busy === 'discover' ? 'busy' : discovery ? 'ok' : failed === 'discover' ? 'fail' : 'idle'">
        <div class="mcp-card-head">
          <span class="mcp-card-icon" aria-hidden="true"><LoaderCircle v-if="busy === 'discover'" :size="20" class="animate-spin" /><Wrench v-else :size="20" /></span>
          <div class="min-w-0 flex-1">
            <h2>{{ t('diagnostics.discovery') }}</h2>
            <p class="mcp-card-status"><i aria-hidden="true" />{{ busy === 'discover' ? t('diagnostics.discovering') : discovery ? t('diagnostics.toolCount', { count: discovery.tools.length }) : failed === 'discover' ? t('diagnostics.failed') : t('diagnostics.notTested') }}</p>
          </div>
        </div>
        <p v-if="discovery" class="mcp-card-meta"><Timer :size="13" aria-hidden="true" />{{ discovery.elapsed_ms }} ms · {{ testedAt ? formatDateTime(testedAt) : '' }}</p>
        <Button type="button" variant="outline" class="mcp-card-action" data-discover-tools :disabled="Boolean(busy)" @click="probe('discover')">{{ t('diagnostics.discover') }}</Button>
      </div>
    </div>
    <Button v-if="busy" type="button" variant="outline" @click="clear">{{ t('diagnostics.cancel') }}</Button>
    <p v-if="error" role="alert" class="rounded-2xl border border-danger/40 bg-danger/10 px-4 py-3 text-sm text-danger-foreground">{{ error }}</p>
    <McpToolCatalog v-if="discovery" :tools="discovery.tools" :dropped-tools="discovery.dropped_tools" />
    <p v-else class="flex items-start gap-2 px-1 text-sm text-muted-foreground"><ShieldCheck :size="16" class="mt-0.5 shrink-0" aria-hidden="true" />{{ t('diagnostics.approvalSeparate') }}</p>
  </section>
</template>

<style scoped>
.mcp-card { --tone: var(--muted-foreground); display: flex; flex-direction: column; gap: 14px; padding: 20px; border: 1px solid color-mix(in oklab, var(--border) 80%, transparent); border-radius: 20px; background: linear-gradient(180deg, color-mix(in oklab, var(--tone) 7%, var(--card)), var(--card) 90px); box-shadow: 0 1px 2px rgb(0 0 0 / 0.04); }
.mcp-card[data-state="ok"] { --tone: var(--success); }
.mcp-card[data-state="fail"] { --tone: var(--danger); }
.mcp-card[data-state="busy"] { --tone: var(--studio-cyan); }
.mcp-card-head { display: flex; align-items: center; gap: 14px; }
.mcp-card-icon { display: grid; place-items: center; flex-shrink: 0; width: 44px; height: 44px; border-radius: 14px; background: color-mix(in oklab, var(--tone) 16%, transparent); color: var(--tone); }
.mcp-card h2 { font-size: 16px; font-weight: 600; }
.mcp-card-status { display: flex; align-items: center; gap: 7px; margin-top: 2px; font-size: 13.5px; color: var(--muted-foreground); }
.mcp-card-status > i { width: 8px; height: 8px; border-radius: 50%; background: var(--tone); }
.mcp-card[data-state="ok"] .mcp-card-status { color: var(--success-foreground); }
.mcp-card[data-state="fail"] .mcp-card-status { color: var(--danger-foreground); }
.mcp-card-meta { display: flex; align-items: center; gap: 6px; font-size: 12.5px; color: var(--muted-foreground); font-variant-numeric: tabular-nums; }
.mcp-card-action { align-self: flex-start; margin-top: auto; border-radius: 999px; }
</style>

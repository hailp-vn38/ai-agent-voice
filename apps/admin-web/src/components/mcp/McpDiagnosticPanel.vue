<script setup lang="ts">
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
    <div class="grid gap-4 lg:grid-cols-2">
      <div class="studio-panel space-y-3 p-4 sm:p-5">
        <h2 class="font-semibold">{{ t('diagnostics.connection') }}</h2>
        <p class="text-sm text-muted-foreground">{{ busy === 'connection' ? t('diagnostics.connecting') : connection ? t('diagnostics.connected') : failed === 'connection' ? t('diagnostics.failed') : t('diagnostics.notTested') }}</p>
        <p v-if="connection" class="text-xs text-muted-foreground">{{ connection.elapsed_ms }} ms · {{ testedAt ? formatDateTime(testedAt) : '' }}</p>
        <Button type="button" variant="outline" data-test-connection :disabled="Boolean(busy)" @click="probe('connection')">{{ t('diagnostics.connect') }}</Button>
      </div>
      <div class="studio-panel space-y-3 p-4 sm:p-5">
        <h2 class="font-semibold">{{ t('diagnostics.discovery') }}</h2>
        <p class="text-sm text-muted-foreground">{{ busy === 'discover' ? t('diagnostics.discovering') : discovery ? t('diagnostics.toolCount', { count: discovery.tools.length }) : failed === 'discover' ? t('diagnostics.failed') : t('diagnostics.notTested') }}</p>
        <p v-if="discovery" class="text-xs text-muted-foreground">{{ discovery.elapsed_ms }} ms · {{ testedAt ? formatDateTime(testedAt) : '' }}</p>
        <Button type="button" variant="outline" data-discover-tools :disabled="Boolean(busy)" @click="probe('discover')">{{ t('diagnostics.discover') }}</Button>
      </div>
    </div>
    <Button v-if="busy" type="button" variant="outline" @click="clear">{{ t('diagnostics.cancel') }}</Button>
    <p v-if="error" role="alert" class="rounded-lg border border-danger/40 p-3 text-sm text-danger">{{ error }}</p>
    <McpToolCatalog v-if="discovery" :tools="discovery.tools" :dropped-tools="discovery.dropped_tools" />
    <p v-else class="text-xs text-muted-foreground">{{ t('diagnostics.approvalSeparate') }}</p>
  </section>
</template>

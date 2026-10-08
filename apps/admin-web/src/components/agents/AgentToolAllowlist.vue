<script setup lang="ts">
import { RefreshCw, ShieldAlert, ShieldCheck } from '@lucide/vue'
import { ref, watch } from 'vue'

import { externalToolsApi, type ObservedExternalTool } from '@/api/external-tools'
import { formatApiError, isApiError } from '@/api/errors'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{ agentId: string }>()
const { t, formatDateTime } = useI18n()
const tools = ref<ObservedExternalTool[]>([])
const error = ref('')
const busy = ref(false)
const loading = ref(false)

async function load() {
  loading.value = true
  error.value = ''
  try {
    tools.value = (await externalToolsApi.list(props.agentId)).items
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    loading.value = false
  }
}

async function review(tool: ObservedExternalTool, allowed: boolean, sensitive: boolean) {
  busy.value = true
  error.value = ''
  try {
    await externalToolsApi.review(props.agentId, {
      server_key: tool.server_key,
      original_name: tool.original_name,
      observed_revision: tool.observed_revision,
      fingerprint: tool.fingerprint,
      allowed,
      sensitive,
    }, tool.revision)
    await load()
  } catch (cause) {
    if (isApiError(cause) && ['revision_conflict', 'contract_conflict'].includes(cause.code)) await load()
    error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}

watch(() => props.agentId, () => { void load() }, { immediate: true })
</script>

<template>
  <section aria-labelledby="external-tool-review-title" class="space-y-4">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h2 id="external-tool-review-title" class="text-lg font-semibold">{{ t('mcp.toolReview') }}</h2>
        <p class="mt-1 text-sm text-muted-foreground">{{ t('mcp.toolReviewHint') }}</p>
      </div>
      <Button variant="outline" size="sm" :disabled="busy || loading" @click="load">
        <RefreshCw class="size-4" aria-hidden="true" />{{ t('common.refresh') }}
      </Button>
    </div>
    <p v-if="error" role="alert" class="rounded-lg border border-danger/40 p-3 text-sm text-danger">{{ error }}</p>
    <p v-if="loading" class="text-sm text-muted-foreground">{{ t('common.loading') }}</p>
    <p v-else-if="!tools.length" class="rounded-lg border border-dashed border-border/70 p-5 text-sm text-muted-foreground">{{ t('mcp.noObservedTools') }}</p>
    <div v-else class="grid gap-3 lg:grid-cols-2">
      <article v-for="tool in tools" :key="`${tool.server_key}/${tool.original_name}`" class="rounded-xl border border-border/70 bg-surface p-4">
        <div class="flex items-start justify-between gap-3">
          <div class="min-w-0">
            <p class="truncate font-semibold" :title="tool.original_name">{{ tool.original_name }}</p>
            <p class="mt-1 font-mono text-xs text-muted-foreground">{{ tool.server_key }}</p>
          </div>
          <span class="flex items-center gap-1 text-xs" :class="tool.allowed && !tool.sensitive ? 'text-success' : 'text-muted-foreground'">
            <ShieldCheck v-if="tool.allowed && !tool.sensitive" class="size-4" aria-hidden="true" />
            <ShieldAlert v-else class="size-4" aria-hidden="true" />
            {{ tool.sensitive ? t('mcp.sensitiveBlocked') : tool.allowed ? t('mcp.approved') : t('mcp.blocked') }}
          </span>
        </div>
        <p v-if="tool.description" class="mt-3 text-sm text-muted-foreground">{{ tool.description }}</p>
        <p class="mt-3 text-xs text-muted-foreground">{{ t('mcp.observedAt') }}: {{ formatDateTime(new Date(tool.observed_at * 1000)) }}</p>
        <details class="mt-3 rounded-lg border border-border/70 p-3 text-xs">
          <summary class="cursor-pointer font-medium">{{ t('mcp.inputSchema') }}</summary>
          <pre class="mt-2 max-h-44 overflow-auto whitespace-pre-wrap break-words font-mono">{{ JSON.stringify(tool.input_schema, null, 2) }}</pre>
        </details>
        <div class="mt-4 flex flex-wrap items-center gap-2">
          <Button size="sm" variant="outline" :disabled="busy || loading || (tool.allowed && !tool.sensitive)" @click="review(tool, true, false)">
            {{ t('mcp.approve') }}
          </Button>
          <Button size="sm" variant="outline" :disabled="busy || loading || (!tool.allowed && !tool.sensitive)" @click="review(tool, false, false)">
            {{ t('mcp.revoke') }}
          </Button>
          <Button v-if="!tool.sensitive" size="sm" variant="outline" :disabled="busy || loading" @click="review(tool, false, true)">
            {{ t('mcp.markSensitive') }}
          </Button>
          <Button v-else size="sm" variant="outline" :disabled="busy || loading" @click="review(tool, false, false)">
            {{ t('mcp.clearSensitive') }}
          </Button>
        </div>
      </article>
    </div>
    <p class="text-xs text-muted-foreground">{{ t('mcp.observationNotice') }}</p>
  </section>
</template>

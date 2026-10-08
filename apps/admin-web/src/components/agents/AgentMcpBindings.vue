<script setup lang="ts">
import { Link2, Plus, RefreshCw, Unlink } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { RouterLink } from 'vue-router'

import { agentsApi } from '@/api/agents'
import { mcpApi } from '@/api/mcp'
import { formatApiError, isApiError } from '@/api/errors'
import type { AgentMcpBinding, AdminMcpServer } from '@/api/types/mcp'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{ agentId: string }>()
const emit = defineEmits<{ changed: [] }>()
const { t } = useI18n()

const servers = ref<AdminMcpServer[]>([])
const bindings = ref<AgentMcpBinding[]>([])
const selectedKey = ref('')
const revision = ref<number>()
const loading = ref(false)
const busy = ref(false)
const error = ref('')
const message = ref('')

const unlinked = computed(() => servers.value.filter((server) => !bindings.value.some((binding) => binding.server_key === server.key)))
const serverByKey = computed(() => new Map(servers.value.map((server) => [server.key, server])))

async function load() {
  loading.value = true
  error.value = ''
  try {
    // Binding endpoint returns { items } only. Agent resource owns revision for If-Match.
    const [agent, result] = await Promise.all([
      agentsApi.get(props.agentId),
      agentsApi.mcpBindings(props.agentId),
    ])
    const catalog: AdminMcpServer[] = []
    let page = 1
    for (; page <= 20; page++) {
      const result = await mcpApi.list({ page, pageSize: 200 })
      catalog.push(...result.items)
      if (result.items.length < result.page_size) break
    }
    if (page > 20) throw new Error(t('mcp.catalogLimit'))
    revision.value = agent.revision
    bindings.value = result.items
    servers.value = catalog
    if (!unlinked.value.some((server) => server.key === selectedKey.value)) selectedKey.value = ''
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    loading.value = false
  }
}

async function changeBinding(serverKey: string, enabled: boolean | null) {
  if (revision.value === undefined) return
  busy.value = true
  error.value = ''
  message.value = ''
  try {
    if (enabled === null) {
      await agentsApi.unlinkMcpServer(props.agentId, serverKey, revision.value)
    } else {
      await agentsApi.bindMcpServer(props.agentId, serverKey, { enabled, required: false }, revision.value)
    }
    await load()
    message.value = t('mcp.bindingSaved')
    emit('changed')
  } catch (cause) {
    if (isApiError(cause) && cause.code === 'revision_conflict') await load()
    error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}

watch(() => props.agentId, () => { void load() }, { immediate: true })
</script>

<template>
  <section class="space-y-4" aria-labelledby="agent-mcp-heading">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h2 id="agent-mcp-heading" class="text-lg font-semibold">{{ t('mcp.bindingsTitle') }}</h2>
        <p class="mt-1 text-sm text-muted-foreground">{{ t('mcp.bindingsHint') }}</p>
      </div>
      <Button size="sm" variant="outline" :disabled="loading || busy" @click="load">
        <RefreshCw class="size-4" aria-hidden="true" />{{ t('common.refresh') }}
      </Button>
    </div>

    <p v-if="error" role="alert" class="rounded-lg border border-danger/40 p-3 text-sm text-danger">{{ error }}</p>
    <p v-if="message" role="status" class="text-sm text-muted-foreground">{{ message }}</p>
    <p v-if="loading" class="text-sm text-muted-foreground">{{ t('common.loading') }}</p>

    <div class="flex flex-wrap gap-2 rounded-xl border border-border/70 bg-surface p-3">
      <label class="min-w-48 flex-1 space-y-1">
        <span class="sr-only">{{ t('mcp.selectServer') }}</span>
        <select v-model="selectedKey" class="admin-input" :disabled="busy || loading || !unlinked.length">
          <option value="">{{ t('mcp.selectServer') }}</option>
          <option v-for="server in unlinked" :key="server.key" :value="server.key">
            {{ server.name }} ({{ server.key }}){{ !server.enabled ? ' — disabled' : '' }}
          </option>
        </select>
      </label>
      <Button :disabled="!selectedKey || loading || busy || revision === undefined" @click="changeBinding(selectedKey, true)">
        <Plus class="size-4" aria-hidden="true" />{{ t('mcp.link') }}
      </Button>
    </div>

    <div v-if="!loading && bindings.length" class="divide-y divide-border/70 overflow-hidden rounded-xl border border-border/70">
      <div v-for="binding in bindings" :key="binding.server_key" class="flex flex-wrap items-center gap-3 p-4">
        <span class="flex size-10 shrink-0 items-center justify-center rounded-xl bg-studio-violet/10 text-studio-violet">
          <Link2 class="size-5" aria-hidden="true" />
        </span>
        <div class="min-w-40 flex-1">
          <p class="truncate font-semibold">{{ serverByKey.get(binding.server_key)?.name ?? binding.server_key }}</p>
          <p class="truncate font-mono text-xs text-muted-foreground">{{ binding.server_key }}</p>
          <p class="mt-1 text-xs text-muted-foreground">
            {{ serverByKey.get(binding.server_key)?.enabled === false ? t('mcp.serverDisabled') : t('mcp.bindingPolicy') }}
          </p>
        </div>
        <span class="text-xs" :class="binding.enabled ? 'text-success' : 'text-muted-foreground'">
          {{ binding.enabled ? t('mcp.enabled') : t('mcp.disabled') }}
        </span>
        <Button size="sm" variant="outline" :disabled="busy || loading" @click="changeBinding(binding.server_key, !binding.enabled)">
          {{ binding.enabled ? t('mcp.disable') : t('mcp.enable') }}
        </Button>
        <Button size="sm" variant="ghost" :disabled="busy || loading" @click="changeBinding(binding.server_key, null)">
          <Unlink class="size-4" aria-hidden="true" />{{ t('mcp.unlink') }}
        </Button>
      </div>
    </div>
    <p v-else-if="!loading" class="rounded-lg border border-dashed border-border/70 p-5 text-sm text-muted-foreground">
      {{ t('mcp.noBindings') }}
    </p>

    <p class="text-xs text-muted-foreground">{{ t('mcp.reviewHint') }}</p>
    <RouterLink to="/mcp" class="inline-flex items-center gap-2 text-sm text-studio-violet hover:underline">
      <Link2 class="size-4" aria-hidden="true" />{{ t('mcp.manageCatalog') }}
    </RouterLink>
  </section>
</template>

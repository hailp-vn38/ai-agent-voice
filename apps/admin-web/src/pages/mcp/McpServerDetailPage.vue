<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { RouterLink, useRoute, useRouter } from 'vue-router'
import { Server, Pencil } from '@lucide/vue'
import { mcpApi } from '@/api/mcp'
import { formatApiError, isApiError } from '@/api/errors'
import type { AdminMcpServer, UpdateMcpServerInput } from '@/api/types/mcp'
import PageHeader from '@/components/admin/PageHeader.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import McpDiagnosticPanel from '@/components/mcp/McpDiagnosticPanel.vue'
import McpServerFormModal from '@/components/mcp/McpServerFormModal.vue'
import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const key = computed(() => typeof route.params.key === 'string' ? route.params.key : '')
const server = ref<AdminMcpServer>()
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const editOpen = ref(false)
const deleteOpen = ref(false)
let controller: AbortController | undefined
const endpoint = computed(() => {
  try { const url = new URL(server.value?.url ?? ''); return `${url.origin}${url.pathname}` }
  catch { return t('common.unknown') }
})
async function load() {
  controller?.abort()
  const pending = new AbortController(); controller = pending
  server.value = undefined; loading.value = true; error.value = ''
  try { const result = await mcpApi.get(key.value, pending.signal); if (!pending.signal.aborted) server.value = result }
  catch (cause) { if (!pending.signal.aborted) error.value = formatApiError(cause) }
  finally { if (controller === pending) loading.value = false }
}
async function update(input: UpdateMcpServerInput) {
  if (!server.value || saving.value) return
  const target = server.value
  saving.value = true; error.value = ''
  try {
    const result = await mcpApi.update(target.key, input, target.revision)
    if (key.value !== target.key) return
    server.value = result; editOpen.value = false
  } catch (cause) {
    if (key.value !== target.key) return
    if (isApiError(cause) && cause.status === 409) { await load(); editOpen.value = false }
    error.value = formatApiError(cause)
  } finally { saving.value = false }
}
async function remove() {
  if (!server.value || saving.value) return
  const target = server.value; saving.value = true; error.value = ''
  try { await mcpApi.remove(target.key, target.revision); if (key.value === target.key) await router.push('/mcp') }
  catch (cause) { if (key.value === target.key) { if (isApiError(cause) && cause.status === 409) await load(); error.value = formatApiError(cause) } }
  finally { saving.value = false }
}
watch(key, () => { editOpen.value = false; deleteOpen.value = false; void load() }, { immediate: true })
onBeforeUnmount(() => controller?.abort())
</script>
<template>
  <main class="mx-auto max-w-6xl space-y-5">
    <RouterLink to="/mcp" class="inline-flex text-sm text-studio-violet hover:underline">← {{ t('mcp.title') }}</RouterLink>
    <section v-if="server" class="studio-panel space-y-3 p-4 sm:p-5">
      <PageHeader :title="server.name" :eyebrow="'Streamable HTTP · ' + (server.enabled ? t('mcp.enabled') : t('mcp.disabled'))">
        <template #actions>
          <Button variant="outline" :disabled="saving" @click="editOpen = true"><Pencil class="size-4" aria-hidden="true" />{{ t('common.edit') }}</Button>
          <ActionMenu :label="t('common.actions')">
            <MenuItem :disabled="saving" @select="update({ enabled: !server.enabled })">{{ server.enabled ? t('mcp.disable') : t('mcp.enable') }}</MenuItem>
            <MenuItem :disabled="saving" variant="danger" @select="deleteOpen = true">{{ t('common.delete') }}</MenuItem>
          </ActionMenu>
        </template>
      </PageHeader>
      <p class="flex items-center gap-2 text-xs text-muted-foreground"><Server class="size-4" aria-hidden="true" /><span class="font-mono">{{ server.key }}</span> · {{ endpoint }} · {{ server.auth.type }} <span v-if="server.credential">· {{ server.credential.masked_key }}</span></p>
      <p v-if="server.url.startsWith('http:') && server.auth.type !== 'none'" class="text-xs text-warning">{{ t('diagnostics.httpWarning') }}</p>
    </section>
    <p v-if="error" role="alert" class="rounded-lg border border-danger/40 p-3 text-sm text-danger">{{ error }}</p>
    <div v-if="loading" class="studio-panel p-8 text-center text-muted-foreground" role="status">{{ t('common.loading') }}</div>
    <template v-if="server">
      <McpDiagnosticPanel :saved-key="server.key" :revision="server.revision" />
      <section class="studio-panel space-y-3 p-4 sm:p-5">
        <h2 class="font-semibold">{{ t('diagnostics.configuration') }}</h2>
        <dl class="grid gap-4 text-sm sm:grid-cols-2">
          <div><dt class="text-xs text-muted-foreground">{{ t('mcp.endpoint') }}</dt><dd class="break-all">{{ endpoint }}</dd></div>
          <div><dt class="text-xs text-muted-foreground">{{ t('mcp.auth') }}</dt><dd>{{ server.auth.type }} {{ server.auth.type === 'header' ? server.auth.header_name : '' }} {{ server.credential?.masked_key ?? '' }} {{ server.credential ? '· v' + server.credential.key_version : '' }}</dd></div>
          <div><dt class="text-xs text-muted-foreground">{{ t('mcp.connectTimeout') }}</dt><dd>{{ server.connect_timeout_ms }} ms</dd></div>
          <div><dt class="text-xs text-muted-foreground">{{ t('mcp.requestTimeout') }}</dt><dd>{{ server.request_timeout_ms }} ms</dd></div>
        </dl>
        <RouterLink to="/agents" class="inline-flex text-sm text-studio-violet hover:underline">{{ t('mcp.manageAgents') }}</RouterLink>
      </section>
      <McpServerFormModal v-model="editOpen" :server="server" :saving="saving" :server-error="error" @save="update" />
      <ConfirmDialog v-model="deleteOpen" :title="t('mcp.deleteTitle', { name: server.name })" :description="t('mcp.deleteHint')" tone="danger" @confirm="remove" />
    </template>
  </main>
</template>

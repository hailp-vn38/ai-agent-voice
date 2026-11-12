<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { RouterLink, useRoute, useRouter } from 'vue-router'
import { ArrowRight, KeyRound, Link2, Pencil, Server, Settings2, Timer } from '@lucide/vue'
import { mcpApi } from '@/api/mcp'
import { formatApiError, isApiError } from '@/api/errors'
import type { AdminMcpServer, UpdateMcpServerInput } from '@/api/types/mcp'
import DetailHero from '@/components/admin/DetailHero.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import McpDiagnosticPanel from '@/components/mcp/McpDiagnosticPanel.vue'
import McpServerFormModal from '@/components/mcp/McpServerFormModal.vue'
import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { useI18n } from '@/composables/useI18n'
import { useErrorToast } from '@/composables/useNotify'
const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const key = computed(() => typeof route.params.key === 'string' ? route.params.key : '')
const server = ref<AdminMcpServer>()
const loading = ref(false)
const saving = ref(false)
const error = ref('')
useErrorToast(() => error.value, { retry: () => load() })
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
  <section class="space-y-5">
    <DetailHero v-if="server" tone="mcp" eyebrow="Streamable HTTP" :title="server.name" :description="endpoint" :back-label="t('mcp.title')" @back="router.push('/mcp')">
      <template #icon><Server :size="28" /></template>
      <template #meta>
        <span class="hero-code"><code class="px-0.5 py-1">{{ server.key }}</code></span>
        <span class="hero-code"><span class="px-0.5 py-1">{{ server.auth.type }}<template v-if="server.credential"> · {{ server.credential.masked_key }}</template></span></span>
      </template>
      <template #actions>
        <span class="inline-flex h-9 items-center gap-2 rounded-full border border-white/30 bg-black/15 px-3 text-sm font-medium">
          <span class="size-2 rounded-full" :class="server.enabled ? 'bg-emerald-300' : 'bg-white/50'" aria-hidden="true" />{{ server.enabled ? t('mcp.enabled') : t('mcp.disabled') }}
        </span>
        <button type="button" class="hero-primary" :disabled="saving" @click="editOpen = true"><Pencil :size="16" aria-hidden="true" />{{ t('common.edit') }}</button>
        <span class="hero-menu">
          <ActionMenu :label="t('common.actions')">
            <MenuItem :disabled="saving" @select="update({ enabled: !server.enabled })">{{ server.enabled ? t('mcp.disable') : t('mcp.enable') }}</MenuItem>
            <MenuItem :disabled="saving" variant="danger" @select="deleteOpen = true">{{ t('common.delete') }}</MenuItem>
          </ActionMenu>
        </span>
      </template>
    </DetailHero>
    <RouterLink v-else to="/mcp" class="inline-flex text-sm text-studio-violet hover:underline">← {{ t('mcp.title') }}</RouterLink>
    <p v-if="server && server.url.startsWith('http:') && server.auth.type !== 'none'" class="rounded-2xl border border-warning/40 bg-warning/10 px-4 py-3 text-sm text-warning-foreground">{{ t('diagnostics.httpWarning') }}</p>
    <div v-if="loading" class="studio-panel !rounded-[20px] p-8 text-center text-muted-foreground" role="status">{{ t('common.loading') }}</div>
    <template v-if="server">
      <div class="grid items-start gap-5 xl:grid-cols-[minmax(0,1.7fr)_minmax(320px,1fr)]">
        <McpDiagnosticPanel :saved-key="server.key" :revision="server.revision" />
        <aside class="studio-panel !rounded-[20px] p-5 sm:p-6">
          <h2 class="flex items-center gap-2 text-base font-semibold"><Settings2 :size="18" class="text-studio-cyan" aria-hidden="true" />{{ t('diagnostics.configuration') }}</h2>
          <dl class="mt-4 divide-y divide-border/60">
            <div class="py-3 first:pt-0"><dt class="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wider text-muted-foreground"><Link2 :size="13" aria-hidden="true" />{{ t('mcp.endpoint') }}</dt><dd class="mt-1.5 break-all font-mono text-[13px]">{{ endpoint }}</dd></div>
            <div class="py-3"><dt class="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wider text-muted-foreground"><KeyRound :size="13" aria-hidden="true" />{{ t('mcp.auth') }}</dt><dd class="mt-1.5 flex flex-wrap items-center gap-2 text-sm"><span class="rounded-md bg-muted px-2 py-0.5 font-medium">{{ server.auth.type }}</span><code v-if="server.auth.type === 'header'" class="text-xs">{{ server.auth.header_name }}</code><code v-if="server.credential?.masked_key" class="text-xs text-muted-foreground">{{ server.credential.masked_key }}</code><span v-if="server.credential" class="text-xs text-muted-foreground">v{{ server.credential.key_version }}</span></dd></div>
            <div class="grid grid-cols-2 gap-4 py-3">
              <div><dt class="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wider text-muted-foreground"><Timer :size="13" aria-hidden="true" />{{ t('mcp.connectTimeout') }}</dt><dd class="mt-1.5 text-lg font-semibold tabular-nums">{{ server.connect_timeout_ms }} <small class="text-xs font-normal text-muted-foreground">ms</small></dd></div>
              <div><dt class="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wider text-muted-foreground"><Timer :size="13" aria-hidden="true" />{{ t('mcp.requestTimeout') }}</dt><dd class="mt-1.5 text-lg font-semibold tabular-nums">{{ server.request_timeout_ms }} <small class="text-xs font-normal text-muted-foreground">ms</small></dd></div>
            </div>
          </dl>
          <RouterLink to="/agents" class="mt-2 inline-flex items-center gap-1.5 rounded-full bg-studio-cyan/10 px-3.5 py-2 text-sm font-medium text-studio-cyan transition-colors hover:bg-studio-cyan/20">{{ t('mcp.manageAgents') }}<ArrowRight :size="15" aria-hidden="true" /></RouterLink>
        </aside>
      </div>
      <McpServerFormModal v-model="editOpen" :server="server" :saving="saving" :server-error="error" @save="update" />
      <ConfirmDialog v-model="deleteOpen" :title="t('mcp.deleteTitle', { name: server.name })" :description="t('mcp.deleteHint')" tone="danger" @confirm="remove" />
    </template>
  </section>
</template>

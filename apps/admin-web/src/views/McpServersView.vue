<script setup lang="ts">
import { ArrowRight, Link2, Pencil, Plus, RefreshCw, Search, Server, ShieldCheck, Trash2 } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { RouterLink } from 'vue-router'

import { mcpApi } from '@/api/mcp'
import { formatApiError, isApiError } from '@/api/errors'
import type { AdminMcpServer, CreateMcpServerInput, UpdateMcpServerInput } from '@/api/types/mcp'
import PageHeader from '@/components/admin/PageHeader.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import McpServerFormModal from '@/components/mcp/McpServerFormModal.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const { t } = useI18n()
const servers = ref<AdminMcpServer[]>([])
const search = ref('')
const stateFilter = ref<'all' | 'enabled' | 'disabled'>('all')
const page = ref(0)
const hasMore = ref(false)
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const notice = ref('')
const formOpen = ref(false)
const editing = ref<AdminMcpServer>()
const deleteTarget = ref<AdminMcpServer>()
const confirmDeleteOpen = computed({
  get: () => Boolean(deleteTarget.value),
  set: (value: boolean) => { if (!value) deleteTarget.value = undefined },
})
const controller = new AbortController()

const visible = computed(() => servers.value.filter((server) => {
  const term = search.value.trim().toLowerCase()
  const matches = !term || [server.name, server.key, server.url].some((part) => part.toLowerCase().includes(term))
  return matches && (stateFilter.value === 'all' || server.enabled === (stateFilter.value === 'enabled'))
}))

/** Do not reflect URL userinfo, query tokens, or fragments in the catalog. */
function safeEndpoint(raw: string) {
  try {
    const url = new URL(raw)
    return `${url.protocol}//${url.host}${url.pathname}`
  } catch {
    return t('common.unknown')
  }
}

async function load(reset = true) {
  if (loading.value) return
  loading.value = true
  error.value = ''
  try {
    const nextPage = reset ? 1 : page.value + 1
    const result = await mcpApi.list({ page: nextPage, pageSize: 50 }, controller.signal)
    if (controller.signal.aborted) return
    servers.value = reset ? result.items : [...servers.value, ...result.items]
    page.value = nextPage
    // Rust doesn't send total or total_pages; a short/empty page is terminal.
    hasMore.value = result.items.length === result.page_size
  } catch (cause) {
    if (!controller.signal.aborted) error.value = formatApiError(cause)
  } finally {
    loading.value = false
  }
}

function createServer() {
  error.value = ''
  editing.value = undefined
  formOpen.value = true
}

async function editServer(server: AdminMcpServer) {
  error.value = ''
  try {
    editing.value = await mcpApi.get(server.key)
    formOpen.value = true
  } catch (cause) {
    error.value = formatApiError(cause)
  }
}

async function saveServer(payload: CreateMcpServerInput | UpdateMcpServerInput) {
  saving.value = true
  error.value = ''
  notice.value = ''
  try {
    if (editing.value) {
      await mcpApi.update(editing.value.key, payload as UpdateMcpServerInput, editing.value.revision)
    } else if ('key' in payload) {
      await mcpApi.create(payload as CreateMcpServerInput)
    }
    formOpen.value = false
    editing.value = undefined
    notice.value = t('mcp.saved')
    await load()
  } catch (cause) {
    error.value = formatApiError(cause)
    if (isApiError(cause) && cause.code === 'revision_conflict') {
      // Keep the form draft and let the operator deliberately retry.
      notice.value = t('mcp.conflict')
    }
  } finally {
    saving.value = false
  }
}

async function toggleServer(server: AdminMcpServer) {
  saving.value = true
  error.value = ''
  notice.value = ''
  try {
    await mcpApi.update(server.key, { enabled: !server.enabled }, server.revision)
    notice.value = t('mcp.changedApproval')
    await load()
  } catch (cause) {
    if (isApiError(cause) && cause.code === 'revision_conflict') await load()
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

async function removeServer() {
  const target = deleteTarget.value
  if (!target) return
  saving.value = true
  error.value = ''
  try {
    await mcpApi.remove(target.key, target.revision)
    deleteTarget.value = undefined
    notice.value = t('mcp.removed')
    await load()
  } catch (cause) {
    if (isApiError(cause) && cause.code === 'revision_conflict') await load()
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

onMounted(() => { void load() })
onBeforeUnmount(() => controller.abort())
</script>

<template>
  <section class="space-y-6">
    <PageHeader :eyebrow="t('mcp.eyebrow')" :title="t('mcp.title')" :description="t('mcp.description')">
      <template #actions>
        <Button variant="outline" :disabled="loading || saving" @click="load()">
          <RefreshCw class="size-4" aria-hidden="true" />{{ t('common.refresh') }}
        </Button>
        <Button :disabled="saving" @click="createServer">
          <Plus class="size-4" aria-hidden="true" />{{ t('mcp.create') }}
        </Button>
      </template>
    </PageHeader>

    <div class="studio-panel grid gap-3 p-4 text-sm sm:grid-cols-3">
      <div class="flex items-start gap-3"><Server class="size-5 text-studio-violet" aria-hidden="true" /><div><p class="font-semibold">{{ t('mcp.step1') }}</p><p class="mt-1 text-xs text-muted-foreground">{{ t('mcp.step1Hint') }}</p></div></div>
      <div class="flex items-start gap-3"><Link2 class="size-5 text-studio-violet" aria-hidden="true" /><div><p class="font-semibold">{{ t('mcp.step2') }}</p><p class="mt-1 text-xs text-muted-foreground">{{ t('mcp.step2Hint') }}</p></div></div>
      <div class="flex items-start gap-3"><ShieldCheck class="size-5 text-studio-cyan" aria-hidden="true" /><div><p class="font-semibold">{{ t('mcp.step3') }}</p><p class="mt-1 text-xs text-muted-foreground">{{ t('mcp.step3Hint') }}</p></div></div>
    </div>

    <div v-if="error" class="rounded-lg border border-danger/40 p-3 text-sm text-danger" role="alert">{{ error }}</div>
    <div v-if="notice" class="rounded-lg border border-border p-3 text-sm text-muted-foreground" role="status">{{ notice }}</div>

    <div class="flex flex-wrap items-center gap-3">
      <label class="relative min-w-60 flex-1 sm:max-w-sm">
        <span class="sr-only">{{ t('mcp.search') }}</span>
        <Search class="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
        <input v-model="search" type="search" class="admin-input pl-9" :placeholder="t('mcp.search')" />
      </label>
      <label class="flex items-center gap-2 text-sm">
        <span class="sr-only">{{ t('mcp.filter') }}</span>
        <select v-model="stateFilter" class="admin-input min-w-36">
          <option value="all">{{ t('common.all') }}</option>
          <option value="enabled">{{ t('mcp.enabled') }}</option>
          <option value="disabled">{{ t('mcp.disabled') }}</option>
        </select>
      </label>
      <span class="text-xs text-muted-foreground">{{ t('mcp.loadedCount', { count: servers.length }) }}</span>
    </div>

    <div v-if="visible.length" class="grid gap-4 lg:grid-cols-2 xl:grid-cols-3">
      <article v-for="server in visible" :key="server.key" class="studio-panel flex flex-col gap-4 p-5">
        <div class="flex items-start justify-between gap-3">
          <span class="flex size-11 shrink-0 items-center justify-center rounded-xl bg-studio-violet/10 text-studio-violet">
            <Server class="size-5" aria-hidden="true" />
          </span>
          <span class="rounded-full border px-2.5 py-1 text-xs" :class="server.enabled ? 'border-success/40 text-success' : 'border-border text-muted-foreground'">
            {{ server.enabled ? t('mcp.enabled') : t('mcp.disabled') }}
          </span>
        </div>
        <div class="min-w-0">
          <h2 class="truncate text-lg font-semibold" :title="server.name">{{ server.name }}</h2>
          <p class="mt-1 truncate font-mono text-xs text-muted-foreground">{{ server.key }}</p>
          <p class="mt-3 truncate text-xs text-muted-foreground" :title="safeEndpoint(server.url)">{{ safeEndpoint(server.url) }}</p>
        </div>
        <div class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <span class="rounded-md bg-surface px-2 py-1">Streamable HTTP</span>
          <span class="rounded-md bg-surface px-2 py-1">{{ t('mcp.auth') }}: {{ server.auth.type }}</span>
        </div>
        <p class="text-xs text-muted-foreground">{{ t('mcp.configNotice') }}</p>
        <div class="mt-auto flex flex-wrap items-center gap-2 border-t border-border/70 pt-4">
          <Button size="sm" variant="outline" :disabled="saving" @click="editServer(server)">
            <Pencil class="size-3.5" aria-hidden="true" />{{ t('common.edit') }}
          </Button>
          <Button size="sm" variant="outline" :disabled="saving" @click="toggleServer(server)">
            {{ server.enabled ? t('mcp.disable') : t('mcp.enable') }}
          </Button>
          <Button size="sm" variant="ghost" :disabled="saving" @click="deleteTarget = server">
            <Trash2 class="size-3.5" aria-hidden="true" />{{ t('common.delete') }}
          </Button>
        </div>
      </article>
    </div>
    <div v-else-if="loading" class="studio-panel p-10 text-center text-sm text-muted-foreground">{{ t('common.loading') }}</div>
    <div v-else class="studio-panel p-10 text-center text-sm text-muted-foreground">{{ t('mcp.empty') }}</div>

    <div v-if="hasMore" class="flex justify-center">
      <Button variant="outline" :disabled="loading" @click="load(false)">{{ t('mcp.loadMore') }}</Button>
    </div>

    <RouterLink to="/agents" class="inline-flex items-center gap-2 text-sm text-studio-violet hover:underline">
      {{ t('mcp.manageAgents') }} <ArrowRight class="size-4" aria-hidden="true" />
    </RouterLink>

    <McpServerFormModal v-model="formOpen" :server="editing" :saving="saving" :server-error="error" @save="saveServer" />
    <ConfirmDialog
      v-model="confirmDeleteOpen"
      :title="t('mcp.deleteTitle', { name: deleteTarget?.name ?? '' })"
      :description="t('mcp.deleteHint')"
      :confirm-label="t('common.delete')"
      tone="danger"
      @confirm="removeServer"
    />
  </section>
</template>

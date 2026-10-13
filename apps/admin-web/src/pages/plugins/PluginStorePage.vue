<script setup lang="ts">
import { Archive, ArrowRight, Boxes, Code2, FlaskConical, PackagePlus, Power, RefreshCw, Search, ShieldAlert, Trash2, UploadCloud } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { agentPluginsApi } from '@/api/agent-plugins'
import { formatApiError, isApiError } from '@/api/errors'
import type { InstalledPlugin, PluginStatus } from '@/api/types/agent-jobs'
import PageHeader from '@/components/admin/PageHeader.vue'
import BaseModal from '@/components/admin/BaseModal.vue'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { useI18n } from '@/composables/useI18n'

const { t } = useI18n()
const items = ref<InstalledPlugin[]>([])
const search = ref('')
const filter = ref<'all' | PluginStatus>('all')
const page = ref(1)
const hasMore = ref(false)
const loading = ref(true)
const busy = ref(false)
const error = ref('')
const notice = ref('')
const backendPending = ref(false)
const selected = ref<InstalledPlugin | null>(null)
const detailsOpen = ref(false)
const installOpen = ref(false)
const selectedFile = ref<File | null>(null)
const fileInput = ref<HTMLInputElement | null>(null)
let controller: AbortController | undefined

const readyCount = computed(() => items.value.filter((item) => item.status === 'ready').length)
const failedCount = computed(() => items.value.filter((item) => item.status === 'error').length)

function statusVariant(status: PluginStatus): 'success' | 'danger' | 'secondary' {
  return status === 'ready' ? 'success' : status === 'error' ? 'danger' : 'secondary'
}
function statusText(status: PluginStatus) {
  return t(('jobUi.pluginStatus.' + status) as 'jobUi.pluginStatus.ready')
}
async function load(nextPage = 1) {
  controller?.abort()
  const pending = new AbortController()
  controller = pending
  loading.value = true
  error.value = ''
  try {
    const result = await agentPluginsApi.list({
      page: nextPage, q: search.value.trim() || undefined,
      status: filter.value === 'all' ? undefined : filter.value,
    }, pending.signal)
    if (pending.signal.aborted) return
    page.value = result.page
    items.value = result.items
    hasMore.value = result.items.length === result.page_size
    backendPending.value = false
  } catch (cause) {
    if (pending.signal.aborted) return
    backendPending.value = isApiError(cause) && cause.status === 404
    error.value = formatApiError(cause)
  } finally {
    if (controller === pending) loading.value = false
  }
}
async function openDetail(item: InstalledPlugin) {
  selected.value = item
  detailsOpen.value = true
  try {
    selected.value = await agentPluginsApi.get(item.id)
  } catch (cause) {
    error.value = formatApiError(cause)
  }
}
async function install() {
  if (!selectedFile.value || busy.value) return
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    await agentPluginsApi.install(selectedFile.value)
    installOpen.value = false
    selectedFile.value = null
    if (fileInput.value) fileInput.value.value = ''
    notice.value = t('jobUi.pluginInstalled')
    await load()
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}
function chooseFile(event: Event) {
  const target = event.target as HTMLInputElement
  const file = target.files?.[0] ?? null
  if (!file) { selectedFile.value = null; return }
  if (!file.name.toLowerCase().endsWith('.zip') || file.size > 20 * 1024 * 1024) {
    selectedFile.value = null
    target.value = ''
    error.value = t('jobUi.invalidZip')
    return
  }
  error.value = ''
  selectedFile.value = file
}
async function action(type: 'test' | 'activate' | 'disable' | 'uninstall') {
  const item = selected.value
  if (!item || busy.value) return
  if (type === 'uninstall' && !window.confirm(t('jobUi.uninstallConfirm'))) return
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    if (type === 'test') {
      const result = await agentPluginsApi.test(item.id, item.revision)
      notice.value = result.message || (result.ok ? t('jobUi.testPassed') : t('jobUi.testFailed'))
    } else if (type === 'uninstall') {
      await agentPluginsApi.uninstall(item.id, item.revision)
      detailsOpen.value = false
      selected.value = null
      notice.value = t('jobUi.uninstalled')
      await load()
    } else {
      selected.value = type === 'activate'
        ? await agentPluginsApi.activate(item.id, item.revision)
        : await agentPluginsApi.disable(item.id, item.revision)
      notice.value = t('jobUi.saved')
      await load(page.value)
    }
  } catch (cause) {
    error.value = formatApiError(cause)
    if (isApiError(cause) && cause.status === 409) {
      try { selected.value = await agentPluginsApi.get(item.id) } catch { /* Keep error visible. */ }
    }
  } finally {
    busy.value = false
  }
}
onMounted(() => { void load() })
onBeforeUnmount(() => controller?.abort())
</script>

<template>
  <section class="space-y-6">
    <PageHeader :title="t('jobUi.plugins')" :description="t('jobUi.pluginsDescription')">
      <template #actions>
        <Button variant="outline" :disabled="loading" @click="load()">
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" />{{ t('jobUi.refresh') }}
        </Button>
        <Button @click="installOpen = true"><PackagePlus class="size-4" />{{ t('jobUi.installZip') }}</Button>
      </template>
    </PageHeader>

    <div v-if="error" role="alert" class="rounded-lg border border-danger/30 bg-danger/5 p-4 text-sm text-danger">
      <p class="font-semibold">{{ backendPending ? t('jobUi.backendPending') : t('jobUi.loadFailed') }}</p>
      <p class="mt-1 break-words">{{ error }}</p>
    </div>
    <p v-if="notice" role="status" class="rounded-lg border border-success/30 bg-success/5 p-3 text-sm text-success-foreground">{{ notice }}</p>

    <div class="grid gap-3 sm:grid-cols-3">
      <div class="studio-panel flex items-center gap-3 p-4">
        <Boxes class="size-6 text-studio-violet" />
        <div><p class="text-xl font-semibold tabular-nums">{{ items.length }}</p><p class="text-xs text-muted-foreground">{{ t('jobUi.pluginCount') }}</p></div>
      </div>
      <div class="studio-panel flex items-center gap-3 p-4">
        <Power class="size-6 text-success" />
        <div><p class="text-xl font-semibold tabular-nums">{{ readyCount }}</p><p class="text-xs text-muted-foreground">{{ t('jobUi.readyCount') }}</p></div>
      </div>
      <div class="studio-panel flex items-center gap-3 p-4">
        <ShieldAlert class="size-6 text-danger" />
        <div><p class="text-xl font-semibold tabular-nums">{{ failedCount }}</p><p class="text-xs text-muted-foreground">{{ t('jobUi.errorCount') }}</p></div>
      </div>
    </div>

    <div class="flex flex-col gap-2 sm:flex-row">
      <form class="relative min-w-0 flex-1 sm:max-w-sm" @submit.prevent="load()">
        <Search class="pointer-events-none absolute left-3 top-3 size-4 text-muted-foreground" />
        <input v-model="search" class="admin-input pl-9" type="search" :aria-label="t('jobUi.searchPlugins')" :placeholder="t('jobUi.searchPlugins')" />
      </form>
      <select v-model="filter" class="admin-input sm:w-44" :aria-label="t('jobUi.filterStatus')" @change="load()">
        <option value="all">{{ t('jobUi.allStatus') }}</option>
        <option value="ready">{{ t('jobUi.pluginStatus.ready') }}</option>
        <option value="disabled">{{ t('jobUi.pluginStatus.disabled') }}</option>
        <option value="error">{{ t('jobUi.pluginStatus.error') }}</option>
      </select>
      <Button variant="outline" @click="load()">{{ t('jobUi.search') }}</Button>
    </div>

    <div v-if="loading" class="studio-panel p-8 text-center text-sm text-muted-foreground" role="status">{{ t('common.loading') }}</div>
    <div v-else-if="items.length" class="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
      <button v-for="item in items" :key="item.id" type="button"
        class="studio-panel min-w-0 p-4 text-left transition-colors hover:border-studio-violet/40 hover:bg-accent/20 focus-visible:outline-2 focus-visible:outline-ring"
        @click="openDetail(item)">
        <div class="flex items-start justify-between gap-2">
          <div class="flex size-10 items-center justify-center rounded-xl bg-studio-violet/10 text-studio-violet"><Archive class="size-5" /></div>
          <Badge :variant="statusVariant(item.status)">{{ statusText(item.status) }}</Badge>
        </div>
        <h2 class="mt-4 truncate font-semibold">{{ item.name }}</h2>
        <p class="mt-1 line-clamp-2 min-h-10 text-sm text-muted-foreground">{{ item.description }}</p>
        <div class="mt-4 flex items-center justify-between border-t pt-3 text-xs text-muted-foreground">
          <span>{{ item.version }} · {{ item.runtime.language }}</span>
          <span class="inline-flex items-center gap-1 text-studio-violet">{{ t('jobUi.details') }}<ArrowRight class="size-3.5" /></span>
        </div>
      </button>
    </div>
    <div v-else class="studio-panel p-10 text-center text-sm text-muted-foreground">
      <Archive class="mx-auto mb-3 size-8 opacity-60" />{{ t('jobUi.noPlugins') }}
    </div>
    <div class="flex justify-end gap-2">
      <Button variant="outline" :disabled="page <= 1 || loading" @click="load(page - 1)">{{ t('jobUi.previous') }}</Button>
      <Button variant="outline" :disabled="!hasMore || loading" @click="load(page + 1)">{{ t('jobUi.next') }}</Button>
    </div>

    <BaseModal v-model="installOpen" :title="t('jobUi.installZip')" :description="t('jobUi.installHint')">
      <div class="space-y-4">
        <div class="rounded-lg border border-dashed p-6 text-center">
          <UploadCloud class="mx-auto size-8 text-studio-violet" />
          <label class="mt-3 block text-sm font-medium" for="plugin-zip">{{ t('jobUi.chooseZip') }}</label>
          <input id="plugin-zip" ref="fileInput" type="file" accept=".zip,application/zip" class="mt-3 block w-full text-sm" @change="chooseFile" />
          <p class="mt-2 text-xs text-muted-foreground">{{ t('jobUi.zipLimit') }}</p>
        </div>
        <p class="text-sm text-muted-foreground">{{ t('jobUi.installSecurity') }}</p>
        <p v-if="selectedFile" class="text-sm font-medium">{{ selectedFile.name }}</p>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <div class="flex justify-end gap-2">
          <Button variant="outline" @click="installOpen = false">{{ t('jobUi.close') }}</Button>
          <Button :disabled="busy || !selectedFile" @click="install"><PackagePlus class="size-4" />{{ t('jobUi.install') }}</Button>
        </div>
      </div>
    </BaseModal>

    <BaseModal v-model="detailsOpen" :title="selected?.name || t('jobUi.details')" :description="selected?.description || ''">
      <div v-if="selected" class="space-y-5 text-sm">
        <div class="flex flex-wrap items-center gap-2">
          <Badge :variant="statusVariant(selected.status)">{{ statusText(selected.status) }}</Badge>
          <span class="text-muted-foreground">{{ selected.version }} · {{ selected.runtime.language }} / {{ selected.runtime.kind }}</span>
        </div>
        <div v-if="selected.last_error_code" class="rounded-lg border border-danger/30 bg-danger/5 p-3 text-danger">
          {{ t('jobUi.lastError') }}: {{ selected.last_error_code }}
        </div>
        <div><h3 class="mb-2 flex items-center gap-2 font-semibold"><Code2 class="size-4" />{{ t('jobUi.operations') }}</h3>
          <div v-for="operation in selected.operations" :key="operation.name" class="border-t py-2">
            <p class="font-mono text-xs">{{ operation.name }} <span class="text-muted-foreground">· {{ operation.replay }}</span></p>
            <p class="mt-1 text-muted-foreground">{{ operation.description }}</p>
          </div>
        </div>
        <div>
          <h3 class="mb-2 flex items-center gap-2 font-semibold"><ShieldAlert class="size-4" />{{ t('jobUi.declaredPermissions') }}</h3>
          <p class="text-muted-foreground">{{ t('jobUi.network') }}: {{ selected.permissions?.network_hosts.join(', ') || '—' }}</p>
          <p class="mt-1 text-muted-foreground">{{ t('jobUi.scratch') }}: {{ selected.permissions?.scratch_storage ? t('jobUi.yes') : t('jobUi.no') }}</p>
          <p class="mt-2 text-xs text-warning">{{ t('jobUi.permissionsHint') }}</p>
        </div>
        <div class="flex flex-wrap justify-end gap-2 border-t pt-4">
          <Button variant="outline" :disabled="busy" @click="action('test')"><FlaskConical class="size-4" />{{ t('jobUi.test') }}</Button>
          <Button v-if="selected.status !== 'disabled'" variant="outline" :disabled="busy" @click="action('disable')">{{ t('jobUi.disable') }}</Button>
          <Button v-if="selected.status !== 'ready'" :disabled="busy" @click="action('activate')">{{ selected.status === 'error' ? t('jobUi.reset') : t('jobUi.activate') }}</Button>
          <Button variant="outline" :disabled="busy" @click="action('uninstall')"><Trash2 class="size-4" />{{ t('jobUi.uninstall') }}</Button>
        </div>
      </div>
    </BaseModal>
  </section>
</template>

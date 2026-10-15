<script setup lang="ts">
import { Archive, ArrowRight, Boxes, LoaderCircle, PackagePlus, RefreshCw, Search, UploadCloud } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { agentPluginsApi } from '@/api/agent-plugins'
import PluginManagePanel from '@/components/plugins/PluginManagePanel.vue'
import { formatApiError, isApiError } from '@/api/errors'
import type { CorePluginView, InstalledPlugin, PluginStatus } from '@/api/types/agent-jobs'
import PageHeader from '@/components/admin/PageHeader.vue'
import BaseModal from '@/components/admin/BaseModal.vue'
import { Button } from '@/components/ui/button'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Badge } from '@/components/ui/badge'
import { useI18n } from '@/composables/useI18n'

const router = useRouter()
const { t } = useI18n()
const activeTab = ref<'available' | 'installed'>('installed')
const items = ref<InstalledPlugin[]>([])
const coreItems = ref<CorePluginView[]>([])
const coreLoading = ref(true)
const coreError = ref('')
const coreBusy = ref<string | null>(null)
const coreManageOpen = ref(false)
const coreManageItem = ref<CorePluginView | null>(null)
const coreManageMode = ref<'settings' | 'functions' | 'runs'>('functions')
let coreController: AbortController | undefined
const search = ref('')
const filter = ref<'all' | PluginStatus>('all')
const page = ref(1)
const total = ref(0)
const hasMore = ref(false)
const loading = ref(true)
const busy = ref(false)
const error = ref('')
const backendPending = ref(false)
const installOpen = ref(false)
const selectedFile = ref<File | null>(null)
const dragging = ref(false)
const fileInput = ref<HTMLInputElement | null>(null)
let controller: AbortController | undefined

const installedCount = computed(() => coreItems.value.length + total.value)
const visibleCoreItems = computed(() => {
  const query = search.value.trim().toLocaleLowerCase()
  return coreItems.value.filter(plugin => {
    const statusMatches = filter.value === 'all' || (filter.value === 'ready' && plugin.enabled)
      || (filter.value === 'disabled' && !plugin.enabled)
    const queryMatches = !query || [plugin.name, plugin.id, ...plugin.operations.map(op => op.name)]
      .some(value => value.toLocaleLowerCase().includes(query))
    return statusMatches && queryMatches
  })
})
const statusVariant = (status: PluginStatus): 'success' | 'danger' | 'secondary' =>
  status === 'ready' ? 'success' : status === 'error' ? 'danger' : 'secondary'
const statusText = (status: PluginStatus) => t(('jobUi.pluginStatus.' + status) as 'jobUi.pluginStatus.ready')

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
    items.value = result.items
    total.value = result.total ?? result.items.length
    page.value = result.page
    hasMore.value = result.total != null
      ? result.page * result.page_size < result.total
      : result.items.length === result.page_size
    backendPending.value = false
  } catch (cause) {
    if (pending.signal.aborted) return
    backendPending.value = isApiError(cause) && cause.status === 404
    error.value = formatApiError(cause)
  } finally {
    if (controller === pending) loading.value = false
  }
}
async function loadCore() {
  coreController?.abort()
  const pending = new AbortController()
  coreController = pending
  coreLoading.value = true
  coreError.value = ''
  try {
    const catalog = await agentPluginsApi.builtinList(pending.signal)
    if (!pending.signal.aborted) {
      // A GET issued before a successful PATCH must never replace its newer CAS revision.
      coreItems.value = catalog.items.map(incoming => {
        const current = coreItems.value.find(item => item.id === incoming.id)
        return current && current.revision > incoming.revision ? current : incoming
      })
    }
  } catch (cause) {
    if (!pending.signal.aborted) coreError.value = formatApiError(cause)
  } finally {
    if (coreController === pending) coreLoading.value = false
  }
}
async function setCoreEnabled(plugin: CorePluginView) {
  if (coreBusy.value) return
  coreBusy.value = plugin.id
  coreError.value = ''
  try {
    const updated = await agentPluginsApi.builtinSetEnabled(plugin.id, !plugin.enabled, plugin.revision)
    coreItems.value = coreItems.value.map(item =>
      item.id === plugin.id && item.revision <= updated.revision ? updated : item,
    )
  } catch (cause) {
    coreError.value = formatApiError(cause)
    if (isApiError(cause) && cause.status === 409) await loadCore()
  } finally {
    coreBusy.value = null
  }
}
function manageCore(plugin: CorePluginView, mode: 'settings' | 'functions' | 'runs') {
  coreManageItem.value = plugin
  coreManageMode.value = mode
  coreManageOpen.value = true
}
function refresh() { void load(); void loadCore() }
function openDetail(id: string) {
  void router.push({ name: 'plugin-detail', params: { id } })
}
function openInstall() {
  selectedFile.value = null
  dragging.value = false
  error.value = ''
  if (fileInput.value) fileInput.value.value = ''
  installOpen.value = true
}
function chooseFile(event: Event) {
  selectFile((event.target as HTMLInputElement).files?.[0] ?? null)
}
function dropFile(event: DragEvent) {
  dragging.value = false
  const file = event.dataTransfer?.files[0]
  if (!file || busy.value) return
  if (fileInput.value) fileInput.value.value = ''
  selectFile(file)
}
function selectFile(file: File | null) {
  if (busy.value) return
  if (!file) { selectedFile.value = null; return }
  if (!file.name.toLowerCase().endsWith('.zip') || file.size > 20 * 1024 * 1024) {
    if (fileInput.value) fileInput.value.value = ''
    selectedFile.value = null
    error.value = t('jobUi.invalidZip')
    return
  }
  error.value = ''
  selectedFile.value = file
}
async function install() {
  if (!selectedFile.value || busy.value) return
  busy.value = true
  error.value = ''
  try {
    const installed = await agentPluginsApi.install(selectedFile.value)
    installOpen.value = false
    selectedFile.value = null
    if (fileInput.value) fileInput.value.value = ''
    await router.push({ name: 'plugin-detail', params: { id: installed.id } })
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}
onMounted(() => { void load(); void loadCore() })
onBeforeUnmount(() => { controller?.abort(); coreController?.abort() })
</script>


<template>
  <section class="space-y-5">
    <PageHeader :title="t('jobUi.plugins')" :description="t('jobUi.pluginsDescription')">
      <template #actions>
        <Button variant="outline" :disabled="loading || coreLoading" @click="refresh">
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading || coreLoading }" />{{ t('jobUi.refresh') }}
        </Button>
        <Button :disabled="busy" @click="openInstall"><PackagePlus class="size-4" />{{ t('jobUi.installZip') }}</Button>
      </template>
    </PageHeader>

    <Tabs v-model="activeTab" class="space-y-5">
      <TabsList>
        <TabsTrigger value="available"><PackagePlus class="size-4" />{{ t('jobUi.notInstalledTab') }}</TabsTrigger>
        <TabsTrigger value="installed">
          <Boxes class="size-4" />{{ t('jobUi.installedTab') }}
          <span class="rounded-full bg-background/75 px-2 py-0.5 text-xs tabular-nums">{{ installedCount }}</span>
        </TabsTrigger>
      </TabsList>

      <TabsContent value="available">
        <div class="studio-panel flex min-h-64 flex-col items-center justify-center gap-3 p-8 text-center">
          <div class="flex size-14 items-center justify-center rounded-2xl bg-studio-violet/10 text-studio-violet">
            <PackagePlus class="size-7" />
          </div>
          <h2 class="text-lg font-semibold">{{ t('jobUi.availableTitle') }}</h2>
          <p class="max-w-lg text-sm text-muted-foreground">{{ t('jobUi.availableHint') }}</p>
          <Button class="mt-2" @click="openInstall"><UploadCloud class="size-4" />{{ t('jobUi.installZip') }}</Button>
        </div>
      </TabsContent>

      <TabsContent value="installed">
        <div class="space-y-4">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div>
              <h2 class="font-semibold">{{ t('jobUi.installedTitle') }}</h2>
              <p class="text-sm text-muted-foreground">{{ t('jobUi.installedHint') }}</p>
            </div>
            <div class="flex items-center gap-2">
              <Badge variant="secondary">CORE · {{ coreItems.length }}</Badge>
              <Badge variant="secondary">{{ t('jobUi.externalLabel') }} · {{ total }}</Badge>
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
          <div v-if="error" role="alert" class="rounded-lg border border-danger/30 bg-danger/5 p-4 text-sm text-danger">
            <p class="font-semibold">{{ backendPending ? t('jobUi.backendPending') : t('jobUi.loadFailed') }}</p><p class="mt-1 break-words">{{ error }}</p>
          </div>
          <p v-if="coreError" role="alert" class="rounded-lg border border-danger/30 bg-danger/5 p-3 text-sm text-danger">{{ coreError }}</p>
          <p v-if="loading || coreLoading" role="status" class="text-sm text-muted-foreground">{{ t('common.loading') }}</p>

          <div v-if="visibleCoreItems.length || items.length" class="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
            <article v-for="plugin in visibleCoreItems" :key="plugin.id" class="studio-panel flex min-h-44 min-w-0 flex-col gap-3 p-4">
              <div class="flex items-start justify-between gap-2">
                <div class="flex size-10 shrink-0 items-center justify-center rounded-xl bg-studio-violet/10 text-studio-violet"><Boxes class="size-5" /></div>
                <div class="flex items-center gap-2">
                  <Badge variant="secondary">CORE</Badge>
                  <Badge :variant="plugin.enabled ? 'success' : 'secondary'">{{ plugin.enabled ? t('jobUi.coreEnabled') : t('jobUi.coreDisabled') }}</Badge>
                </div>
              </div>
              <div class="min-w-0">
                <h3 class="truncate font-semibold">{{ plugin.name }}</h3>
                <p class="mt-0.5 text-xs text-muted-foreground">{{ plugin.id }} · {{ plugin.runtime.language }}</p>
                <p class="mt-2 line-clamp-2 text-xs text-muted-foreground">{{ plugin.operations.map(op => op.name).join(' · ') }}</p>
              </div>
              <div class="mt-auto flex items-center justify-between gap-2 border-t pt-3">
                <div class="flex flex-wrap items-center gap-2">
                  <span class="text-xs text-muted-foreground">{{ plugin.operations.length }} ops</span>
                  <Button size="sm" variant="outline" @click="manageCore(plugin, 'settings')">Settings</Button>
                  <Button size="sm" variant="outline" :disabled="!plugin.enabled" @click="manageCore(plugin, 'functions')">Functions</Button>
                  <Button size="sm" variant="outline" @click="manageCore(plugin, 'runs')">Runs</Button>
                </div>
                <Button size="sm" variant="outline" :aria-label="(plugin.enabled ? t('jobUi.coreDisable') : t('jobUi.coreEnable')) + ' ' + plugin.name"
                  :disabled="coreBusy !== null" @click="setCoreEnabled(plugin)">
                  {{ coreBusy === plugin.id ? t('jobUi.coreSaving') : plugin.enabled ? t('jobUi.coreDisable') : t('jobUi.coreEnable') }}
                </Button>
              </div>
            </article>
            <button v-for="item in items" :key="item.id" type="button"
              class="studio-panel flex min-h-44 min-w-0 flex-col gap-3 p-4 text-left transition-colors hover:border-studio-violet/40 hover:bg-accent/20 focus-visible:outline-2 focus-visible:outline-ring"
              @click="openDetail(item.id)">
              <div class="flex items-start justify-between gap-2">
                <div class="flex size-10 items-center justify-center rounded-xl bg-studio-violet/10 text-studio-violet"><Archive class="size-5" /></div>
                <div class="flex items-center gap-2">
                  <Badge variant="secondary">{{ t('jobUi.externalLabel') }}</Badge>
                  <Badge :variant="statusVariant(item.status)">{{ statusText(item.status) }}</Badge>
                </div>
              </div>
              <div class="min-w-0">
                <h3 class="truncate font-semibold">{{ item.name }}</h3>
                <p class="mt-1 line-clamp-2 text-sm text-muted-foreground">{{ item.description }}</p>
              </div>
              <div class="mt-auto flex items-center justify-between gap-2 border-t pt-3 text-xs text-muted-foreground">
                <span>{{ item.version }} · {{ item.runtime.language }} · {{ item.operations.length }} ops</span>
                <span class="inline-flex items-center gap-1 text-studio-violet">{{ t('jobUi.details') }}<ArrowRight class="size-3.5" /></span>
              </div>
            </button>
          </div>
          <div v-else-if="!loading && !coreLoading" class="studio-panel p-10 text-center text-sm text-muted-foreground">
            <Archive class="mx-auto mb-3 size-8 opacity-60" />{{ t('jobUi.noPlugins') }}
          </div>
          <div v-if="page > 1 || hasMore" class="flex justify-end gap-2">
            <Button variant="outline" :disabled="page <= 1 || loading" @click="load(page - 1)">{{ t('jobUi.previous') }}</Button>
            <Button variant="outline" :disabled="!hasMore || loading" @click="load(page + 1)">{{ t('jobUi.next') }}</Button>
          </div>
        </div>
      </TabsContent>
    </Tabs>

    <BaseModal v-model="coreManageOpen" :title="coreManageItem?.name || 'Core Plugin'">
      <PluginManagePanel v-if="coreManageItem" :key="coreManageItem.id + coreManageMode"
        :plugin-key="coreManageItem.id"
        :operations="coreManageItem.operations.map(operation => operation.name)"
        :enabled="coreManageItem.enabled" :mode="coreManageMode" />
    </BaseModal>
    <BaseModal v-model="installOpen" :title="t('jobUi.installZip')" :description="t('jobUi.installHint')" :close-disabled="busy">
      <div class="space-y-4" :aria-busy="busy">
        <div class="rounded-lg border border-dashed p-6 text-center transition-colors"
          :class="{ 'border-studio-violet bg-studio-violet/5': dragging }"
          @dragover.prevent="dragging = !busy" @dragleave.prevent="dragging = false" @drop.prevent="dropFile">
          <UploadCloud class="mx-auto size-8 text-studio-violet" />
          <label class="mt-3 block text-sm font-medium" for="plugin-zip">{{ t('jobUi.chooseZip') }}</label>
          <p class="mt-1 text-sm text-muted-foreground">{{ t('jobUi.dropZip') }}</p>
          <input id="plugin-zip" ref="fileInput" type="file" accept=".zip,application/zip" :disabled="busy" class="mt-3 block w-full text-sm" @change="chooseFile" />
          <p class="mt-2 text-xs text-muted-foreground">{{ t('jobUi.zipLimit') }}</p>
        </div>
        <p class="text-sm text-muted-foreground">{{ t('jobUi.installSecurity') }}</p>
        <p v-if="selectedFile" class="text-sm font-medium">{{ selectedFile.name }}</p>
        <p v-if="busy" role="status" class="flex items-center gap-2 text-sm text-muted-foreground">
          <LoaderCircle class="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />{{ t('jobUi.installing') }}
        </p>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <div class="flex justify-end gap-2">
          <Button variant="outline" :disabled="busy" @click="installOpen = false">{{ t('jobUi.close') }}</Button>
          <Button :disabled="busy || !selectedFile" @click="install">
            <LoaderCircle v-if="busy" class="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />
            <PackagePlus v-else class="size-4" />{{ busy ? t('jobUi.installingShort') : t('jobUi.install') }}
          </Button>
        </div>
      </div>
    </BaseModal>
  </section>
</template>

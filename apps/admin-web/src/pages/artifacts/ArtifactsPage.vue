<script setup lang="ts">
import {
  Database,
  Download,
  Eye,
  FileCode2,
  FileSpreadsheet,
  FileText,
  Film,
  FolderOpen,
  HardDrive,
  Image as ImageIcon,
  LayoutGrid,
  List,
  Music,
  Play,
  RefreshCw,
  Search,
  Sparkles,
  X,
} from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { agentJobsApi } from '@/api/agent-jobs'
import { formatApiError, isApiError } from '@/api/errors'
import { listenJobChanges } from '@/api/job-updates'
import type {
  AdminArtifactItem,
  ArtifactCatalogSummary,
  ArtifactKind,
  ArtifactRole,
} from '@/api/types/agent-jobs'
import PageHeader from '@/components/admin/PageHeader.vue'
import ArtifactPreviewDialog, {
  artifactPreview,
} from '@/components/jobs/artifacts/ArtifactPreviewDialog.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useAuthStore } from '@/stores/auth'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()
const auth = useAuthStore()

const items = ref<AdminArtifactItem[]>([])
const summary = ref<ArtifactCatalogSummary>({
  total_artifacts: 0,
  primary_artifacts: 0,
  intermediate_artifacts: 0,
  report_artifacts: 0,
  image_artifacts: 0,
  video_artifacts: 0,
  audio_artifacts: 0,
  document_artifacts: 0,
  data_artifacts: 0,
  other_artifacts: 0,
  total_size_bytes: 0,
})
const search = ref('')
const kindFilter = ref<'all' | ArtifactKind>('all')
const roleFilter = ref<'all' | ArtifactRole>('all')
const jobIdFilter = ref<string>(typeof route.query.job_id === 'string' ? route.query.job_id : '')
const viewMode = ref<'grid' | 'table'>('grid')
const page = ref(1)
const pageSize = ref(24)
const total = ref(0)
const loading = ref(true)
const error = ref('')
const backendPending = ref(false)
const selectedArtifact = ref<AdminArtifactItem | null>(null)
const downloadingId = ref<string | null>(null)

const mediaBlobUrls = ref<Record<string, string>>({})
let activeBlobUrls: Record<string, string> = {}
let blobGeneration = 0

let controller: AbortController | undefined
let streamController: AbortController | undefined
let refreshTimer: ReturnType<typeof setTimeout> | undefined
let reconciling = false
let dirty = false
let mounted = false

const kindTabs = computed(() => [
  { key: 'all' as const, label: t('artifactsUi.kind.all'), count: summary.value.total_artifacts },
  { key: 'report' as const, label: t('artifactsUi.kind.report'), count: summary.value.report_artifacts },
  { key: 'image' as const, label: t('artifactsUi.kind.image'), count: summary.value.image_artifacts },
  { key: 'video' as const, label: t('artifactsUi.kind.video'), count: summary.value.video_artifacts },
  { key: 'audio' as const, label: t('artifactsUi.kind.audio'), count: summary.value.audio_artifacts },
  { key: 'document' as const, label: t('artifactsUi.kind.document'), count: summary.value.document_artifacts },
  { key: 'data' as const, label: t('artifactsUi.kind.data'), count: summary.value.data_artifacts },
])

const mediaAssetsCount = computed(
  () =>
    summary.value.image_artifacts +
    summary.value.video_artifacts +
    summary.value.audio_artifacts,
)

const hasMore = computed(() => page.value * pageSize.value < total.value)

const selectedIndex = computed(() =>
  selectedArtifact.value
    ? items.value.findIndex(entry => entry.id === selectedArtifact.value?.id)
    : -1,
)

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'
  if (bytes < 1024) return `${bytes} B`
  const kib = bytes / 1024
  if (kib < 1024) return `${kib.toFixed(1)} KiB`
  const mib = kib / 1024
  if (mib < 1024) return `${mib.toFixed(2)} MiB`
  return `${(mib / 1024).toFixed(2)} GiB`
}

function formatDate(value: string): string {
  const num = Number(value)
  const stamp =
    Number.isFinite(num) && num > 0
      ? new Date(num < 1_000_000_000_000 ? num * 1000 : num)
      : new Date(value)
  return Number.isFinite(stamp.getTime()) ? stamp.toLocaleString() : value
}

function kindLabel(kind: ArtifactKind): string {
  return t(('artifactsUi.kind.' + kind) as 'artifactsUi.kind.report')
}

function roleLabel(role: ArtifactRole): string {
  return role === 'primary' ? t('artifactsUi.role.primary') : t('artifactsUi.role.intermediate')
}

function fileExtension(item: AdminArtifactItem): string {
  const name = item.filename || item.name || ''
  const dot = name.lastIndexOf('.')
  if (dot > 0 && dot < name.length - 1) {
    return name.slice(dot + 1).toUpperCase()
  }
  const mime = item.mime_type || item.media_type || ''
  const slash = mime.indexOf('/')
  return slash >= 0 ? mime.slice(slash + 1).toUpperCase() : item.kind.toUpperCase()
}

function kindIcon(kind: ArtifactKind) {
  switch (kind) {
    case 'report':
      return FileText
    case 'image':
      return ImageIcon
    case 'video':
      return Film
    case 'audio':
      return Music
    case 'document':
      return FileCode2
    case 'data':
      return FileSpreadsheet
    default:
      return Database
  }
}

function canPreview(item: AdminArtifactItem): boolean {
  const preview = artifactPreview(item)
  return Boolean(item.job_id) && preview.kind !== 'download' && item.size_bytes >= 0 && item.size_bytes <= preview.limit
}

function openArtifactModal(item: AdminArtifactItem) {
  selectedArtifact.value = item
}

function selectAdjacent(offset: -1 | 1) {
  const nextIdx = selectedIndex.value + offset
  const target = items.value[nextIdx]
  if (target) {
    selectedArtifact.value = target
  }
}

function revokeAllBlobs() {
  for (const url of Object.values(activeBlobUrls)) {
    if (url.startsWith('blob:')) URL.revokeObjectURL(url)
  }
  activeBlobUrls = {}
  mediaBlobUrls.value = {}
}

async function hydrateInlineMediaBlobs(loadedItems: AdminArtifactItem[]) {
  const gen = ++blobGeneration
  if (typeof agentJobsApi.artifact !== 'function') return
  const inlineCandidates = loadedItems.filter(
    item =>
      (item.kind === 'image' || item.kind === 'video') &&
      canPreview(item) &&
      item.size_bytes > 0 &&
      item.size_bytes <= 16 * 1024 * 1024,
  )
  const keepIds = new Set(inlineCandidates.map(item => item.id))
  for (const [id, url] of Object.entries(activeBlobUrls)) {
    if (!keepIds.has(id)) {
      if (url.startsWith('blob:')) URL.revokeObjectURL(url)
      delete activeBlobUrls[id]
    }
  }
  mediaBlobUrls.value = { ...activeBlobUrls }

  await Promise.all(
    inlineCandidates.map(async item => {
      if (activeBlobUrls[item.id]) return
      try {
        const blob = await agentJobsApi.artifact(item.job_id, item.id)
        if (gen !== blobGeneration || !mounted) return
        const mime = (item.media_type || item.mime_type || '').trim() || undefined
        const url = URL.createObjectURL(mime ? new Blob([blob], { type: mime }) : blob)
        activeBlobUrls = { ...activeBlobUrls, [item.id]: url }
        mediaBlobUrls.value = { ...activeBlobUrls }
      } catch {
        // Ignore thumbnail fetch failure; user can still click card to open modal or download
      }
    }),
  )
}

async function load(nextPage = page.value) {
  controller?.abort()
  const pending = new AbortController()
  controller = pending
  loading.value = true
  error.value = ''
  try {
    const result = await agentJobsApi.listArtifacts(
      {
        page: nextPage,
        page_size: pageSize.value,
        kind: kindFilter.value === 'all' ? undefined : kindFilter.value,
        role: roleFilter.value === 'all' ? undefined : roleFilter.value,
        job_id: jobIdFilter.value.trim() || undefined,
        q: search.value.trim() || undefined,
      },
      pending.signal,
    )
    if (pending.signal.aborted) return
    items.value = result.items
    total.value = result.total
    page.value = result.page
    summary.value = result.summary
    backendPending.value = false
    void hydrateInlineMediaBlobs(result.items)
  } catch (cause) {
    if (pending.signal.aborted) return
    backendPending.value = isApiError(cause) && cause.status === 404
    error.value = formatApiError(cause)
  } finally {
    if (controller === pending) loading.value = false
  }
}

function selectKind(nextKind: 'all' | ArtifactKind) {
  if (kindFilter.value === nextKind) return
  kindFilter.value = nextKind
  void load(1)
}

function clearJobFilter() {
  jobIdFilter.value = ''
  if (route.query.job_id) {
    void router.replace({ query: { ...route.query, job_id: undefined } })
  }
  void load(1)
}

function openJob(jobId: string) {
  selectedArtifact.value = null
  void router.push({ name: 'job-detail', params: { jobId } })
}

async function downloadArtifact(item: AdminArtifactItem) {
  if (!item.job_id || downloadingId.value) return
  downloadingId.value = item.id
  error.value = ''
  try {
    const data = await agentJobsApi.artifact(item.job_id, item.id)
    const url = URL.createObjectURL(data)
    const link = document.createElement('a')
    link.href = url
    link.download = item.filename || item.name || item.id
    link.click()
    window.setTimeout(() => URL.revokeObjectURL(url), 1000)
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    downloadingId.value = null
  }
}

function reconcile() {
  dirty = true
  if (refreshTimer || reconciling) return
  refreshTimer = setTimeout(() => {
    refreshTimer = undefined
    if (reconciling) return
    void (async () => {
      reconciling = true
      try {
        while (dirty && mounted) {
          dirty = false
          await load(page.value)
        }
      } finally {
        reconciling = false
        if (dirty && mounted) reconcile()
      }
    })()
  }, 300)
}

function startStream() {
  streamController?.abort()
  if (!mounted) return
  streamController = new AbortController()
  void listenJobChanges(
    {
      onReady: reconcile,
      onChanged: reconcile,
      onResync: reconcile,
    },
    streamController.signal,
  )
}

watch(
  () => route.query.job_id,
  nextJobId => {
    const normalized = typeof nextJobId === 'string' ? nextJobId : ''
    if (normalized !== jobIdFilter.value) {
      jobIdFilter.value = normalized
      void load(1)
    }
  },
)

watch(() => auth.adminToken, startStream)

onMounted(() => {
  mounted = true
  void load(1)
  startStream()
})

onBeforeUnmount(() => {
  mounted = false
  blobGeneration++
  controller?.abort()
  streamController?.abort()
  if (refreshTimer) clearTimeout(refreshTimer)
  revokeAllBlobs()
})
</script>

<template>
  <section class="space-y-6">
    <PageHeader :title="t('artifactsUi.title')" :description="t('artifactsUi.description')">
      <template #actions>
        <div class="inline-flex rounded-lg border bg-muted/30 p-0.5">
          <button
            type="button"
            class="inline-flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-xs font-medium transition-colors"
            :class="viewMode === 'grid' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'"
            :aria-label="t('artifactsUi.viewGrid')"
            data-view-mode="grid"
            @click="viewMode = 'grid'"
          >
            <LayoutGrid class="size-3.5" />{{ t('artifactsUi.viewGrid') }}
          </button>
          <button
            type="button"
            class="inline-flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-xs font-medium transition-colors"
            :class="viewMode === 'table' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'"
            :aria-label="t('artifactsUi.viewTable')"
            data-view-mode="table"
            @click="viewMode = 'table'"
          >
            <List class="size-3.5" />{{ t('artifactsUi.viewTable') }}
          </button>
        </div>
        <Button variant="outline" :disabled="loading" @click="load(page)">
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" />{{ t('jobUi.refresh') }}
        </Button>
      </template>
    </PageHeader>

    <div v-if="error" role="alert" class="rounded-lg border border-danger/30 bg-danger/5 p-4 text-sm text-danger">
      <p class="font-medium">{{ backendPending ? t('jobUi.backendPending') : t('jobUi.loadFailed') }}</p>
      <p class="mt-1 break-words">{{ error }}</p>
    </div>

    <div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-4" data-artifact-summary-cards>
      <div class="studio-panel flex items-center gap-3 p-4">
        <FolderOpen class="size-6 text-studio-violet" />
        <div>
          <p class="text-xl font-semibold tabular-nums" data-summary-total>{{ summary.total_artifacts }}</p>
          <p class="text-xs text-muted-foreground">{{ t('artifactsUi.totalFiles') }}</p>
        </div>
      </div>
      <div class="studio-panel flex items-center gap-3 p-4">
        <Sparkles class="size-6 text-success" />
        <div>
          <p class="text-xl font-semibold tabular-nums" data-summary-primary>{{ summary.primary_artifacts }}</p>
          <p class="text-xs text-muted-foreground">{{ t('artifactsUi.primaryDeliverables') }}</p>
        </div>
      </div>
      <div class="studio-panel flex items-center gap-3 p-4">
        <ImageIcon class="size-6 text-studio-cyan" />
        <div>
          <p class="text-xl font-semibold tabular-nums" data-summary-media>{{ mediaAssetsCount }}</p>
          <p class="text-xs text-muted-foreground">{{ t('artifactsUi.mediaAssets') }}</p>
        </div>
      </div>
      <div class="studio-panel flex items-center gap-3 p-4">
        <HardDrive class="size-6 text-studio-violet" />
        <div>
          <p class="text-xl font-semibold tabular-nums" data-summary-bytes>{{ formatBytes(summary.total_size_bytes) }}</p>
          <p class="text-xs text-muted-foreground">{{ t('artifactsUi.totalStorage') }}</p>
        </div>
      </div>
    </div>

    <div class="flex flex-wrap items-center gap-1.5 border-b pb-3" role="tablist" data-artifact-kind-tabs>
      <button
        v-for="tab in kindTabs"
        :key="tab.key"
        type="button"
        role="tab"
        :aria-selected="kindFilter === tab.key"
        :data-kind-tab="tab.key"
        class="inline-flex cursor-pointer items-center gap-1.5 rounded-lg border px-3 py-1.5 text-xs font-medium transition-colors"
        :class="
          kindFilter === tab.key
            ? 'border-studio-violet/50 bg-studio-violet/15 text-foreground'
            : 'border-transparent bg-muted/40 text-muted-foreground hover:bg-muted/70 hover:text-foreground'
        "
        @click="selectKind(tab.key)"
      >
        <span>{{ tab.label }}</span>
        <span class="rounded-full bg-background/80 px-1.5 py-0.2 font-mono text-[11px] tabular-nums">
          {{ tab.count }}
        </span>
      </button>
    </div>

    <div class="flex flex-col gap-2 sm:flex-row sm:items-center">
      <form class="relative min-w-0 flex-1 sm:max-w-md" @submit.prevent="load(1)">
        <Search class="pointer-events-none absolute left-3 top-3 size-4 text-muted-foreground" />
        <input
          v-model="search"
          class="admin-input pl-9"
          type="search"
          data-artifact-search
          :placeholder="t('artifactsUi.searchPlaceholder')"
          :aria-label="t('artifactsUi.searchPlaceholder')"
        />
      </form>
      <select
        v-model="roleFilter"
        class="admin-input sm:w-52"
        data-artifact-role-filter
        :aria-label="t('artifactsUi.role.all')"
        @change="load(1)"
      >
        <option value="all">{{ t('artifactsUi.role.all') }}</option>
        <option value="primary">{{ t('artifactsUi.role.primary') }}</option>
        <option value="intermediate">{{ t('artifactsUi.role.intermediate') }}</option>
      </select>
      <Button variant="outline" @click="load(1)">{{ t('jobUi.search') }}</Button>
      <Button
        v-if="jobIdFilter"
        variant="outline"
        size="sm"
        data-clear-job-filter
        @click="clearJobFilter"
      >
        <X class="size-3.5" />{{ t('artifactsUi.clearJobFilter') }}: {{ jobIdFilter }}
      </Button>
    </div>

    <div v-if="loading" class="studio-panel p-8 text-center text-sm text-muted-foreground" role="status">
      {{ t('common.loading') }}
    </div>

    <div
      v-else-if="items.length && viewMode === 'grid'"
      class="grid gap-4 sm:grid-cols-2 md:grid-cols-3 xl:grid-cols-4"
      data-artifact-grid
    >
      <article
        v-for="item in items"
        :key="item.id"
        role="button"
        tabindex="0"
        data-artifact-card
        :data-artifact-kind="item.kind"
        class="studio-panel group flex cursor-pointer flex-col justify-between overflow-hidden transition-all duration-200 hover:-translate-y-0.5 hover:border-studio-violet/50 hover:shadow-lg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-studio-violet"
        @click="openArtifactModal(item)"
        @keydown.enter.prevent="openArtifactModal(item)"
        @keydown.space.prevent="openArtifactModal(item)"
      >
        <div class="relative aspect-[16/10] w-full overflow-hidden border-b bg-muted/25">
          <img
            v-if="item.kind === 'image' && mediaBlobUrls[item.id]"
            :src="mediaBlobUrls[item.id]"
            :alt="item.filename || item.name"
            loading="lazy"
            class="h-full w-full object-cover transition-transform duration-300 group-hover:scale-105"
            data-artifact-inline-image
          />

          <div
            v-else-if="item.kind === 'video' && mediaBlobUrls[item.id]"
            class="relative h-full w-full bg-black/40"
          >
            <video
              :src="mediaBlobUrls[item.id]"
              preload="metadata"
              muted
              playsinline
              class="pointer-events-none h-full w-full object-cover transition-transform duration-300 group-hover:scale-105"
              data-artifact-inline-video
            />
            <div class="pointer-events-none absolute inset-0 flex items-center justify-center bg-black/25">
              <div class="grid size-11 place-items-center rounded-full bg-background/90 text-foreground shadow-md">
                <Play class="size-5 fill-current" />
              </div>
            </div>
          </div>

          <div
            v-else-if="item.kind === 'audio'"
            class="flex h-full w-full flex-col items-center justify-center gap-2.5 bg-gradient-to-br from-studio-cyan/10 via-transparent to-studio-violet/10 p-4"
          >
            <div class="grid size-12 place-items-center rounded-full bg-studio-cyan/15 text-studio-cyan">
              <Music class="size-6" />
            </div>
            <span class="rounded bg-background/80 px-2 py-0.5 font-mono text-[10px] font-semibold tracking-wider text-muted-foreground">
              {{ fileExtension(item) }}
            </span>
          </div>

          <div
            v-else
            class="flex h-full w-full flex-col items-center justify-center gap-2 bg-gradient-to-br from-studio-violet/8 via-transparent to-muted/30 p-4 text-center"
          >
            <div class="grid size-12 place-items-center rounded-xl bg-background/80 text-studio-violet shadow-sm">
              <component :is="kindIcon(item.kind)" class="size-6" />
            </div>
            <span class="rounded bg-background/80 px-2 py-0.5 font-mono text-[10px] font-semibold tracking-wider text-muted-foreground">
              {{ fileExtension(item) }}
            </span>
          </div>

          <div class="pointer-events-none absolute left-2.5 top-2.5 flex items-center gap-1.5">
            <Badge variant="secondary" class="bg-background/90 text-[10px] backdrop-blur-xs">
              {{ kindLabel(item.kind) }}
            </Badge>
            <Badge
              v-if="item.role === 'primary'"
              variant="success"
              class="text-[10px]"
            >
              {{ roleLabel(item.role) }}
            </Badge>
          </div>

          <div
            class="pointer-events-none absolute inset-0 flex items-center justify-center bg-black/35 opacity-0 transition-opacity duration-200 group-hover:opacity-100"
          >
            <span class="inline-flex items-center gap-1.5 rounded-full bg-background/95 px-3 py-1.5 text-xs font-medium text-foreground shadow-md">
              <Eye class="size-3.5 text-studio-violet" />{{ t('artifactsUi.clickToView') }}
            </span>
          </div>
        </div>

        <div class="flex flex-1 flex-col justify-between gap-2.5 p-3.5">
          <div class="min-w-0">
            <h3
              class="truncate text-sm font-semibold transition-colors group-hover:text-studio-violet"
              :title="item.filename || item.name"
            >
              {{ item.filename || item.name || item.id }}
            </h3>
            <p class="mt-0.5 truncate text-xs text-muted-foreground" :title="item.job_title || item.job_goal || item.job_id">
              {{ item.job_title || item.job_goal || item.job_id }}
            </p>
          </div>

          <div class="flex items-center justify-between gap-2 border-t pt-2 text-[11px] text-muted-foreground">
            <span class="truncate font-mono">
              {{ formatBytes(item.size_bytes) }}
              <span v-if="item.producer_agent_id"> · {{ item.producer_agent_id }}</span>
            </span>
            <button
              type="button"
              data-artifact-download-btn
              class="inline-flex size-7 shrink-0 cursor-pointer items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
              :title="t('jobUi.download')"
              :aria-label="t('jobUi.download')"
              :disabled="downloadingId === item.id"
              @click.stop="downloadArtifact(item)"
            >
              <Download class="size-3.5" />
            </button>
          </div>
        </div>
      </article>
    </div>

    <div v-else-if="items.length && viewMode === 'table'" class="studio-panel overflow-hidden" data-artifact-table>
      <div class="overflow-x-auto">
        <table class="w-full text-left text-sm">
          <thead class="border-b bg-muted/40 text-xs text-muted-foreground">
            <tr>
              <th class="px-4 py-3 font-medium">{{ t('artifactsUi.file') }}</th>
              <th class="px-4 py-3 font-medium">{{ t('artifactsUi.type') }}</th>
              <th class="px-4 py-3 font-medium">{{ t('artifactsUi.size') }}</th>
              <th class="px-4 py-3 font-medium">{{ t('artifactsUi.producedBy') }}</th>
              <th class="px-4 py-3 font-medium">{{ t('artifactsUi.job') }}</th>
              <th class="px-4 py-3 font-medium">{{ t('jobUi.createdAt') }}</th>
              <th class="px-4 py-3 text-right font-medium">{{ t('common.actions') }}</th>
            </tr>
          </thead>
          <tbody class="divide-y">
            <tr
              v-for="item in items"
              :key="item.id"
              class="cursor-pointer transition-colors hover:bg-muted/30"
              data-artifact-row
              @click="openArtifactModal(item)"
            >
              <td class="max-w-xs px-4 py-3.5">
                <p class="truncate font-medium" :title="item.filename || item.name">
                  {{ item.filename || item.name || item.id }}
                </p>
                <p class="truncate font-mono text-[11px] text-muted-foreground">{{ item.mime_type }}</p>
              </td>
              <td class="whitespace-nowrap px-4 py-3.5">
                <div class="flex items-center gap-1.5">
                  <Badge variant="secondary">{{ kindLabel(item.kind) }}</Badge>
                  <Badge :variant="item.role === 'primary' ? 'success' : 'outline'">
                    {{ roleLabel(item.role) }}
                  </Badge>
                </div>
              </td>
              <td class="whitespace-nowrap px-4 py-3.5 font-mono text-xs tabular-nums">
                {{ formatBytes(item.size_bytes) }}
              </td>
              <td class="whitespace-nowrap px-4 py-3.5 font-mono text-xs">
                <span>{{ item.producer_agent_id || '—' }}</span>
                <span v-if="item.producer_tool_name" class="text-muted-foreground">
                  ({{ item.producer_tool_name }})
                </span>
              </td>
              <td class="max-w-xs px-4 py-3.5">
                <span class="truncate font-medium text-studio-violet">
                  {{ item.job_title || item.job_goal || item.job_id }}
                </span>
              </td>
              <td class="whitespace-nowrap px-4 py-3.5 text-xs text-muted-foreground">
                {{ formatDate(item.created_at) }}
              </td>
              <td class="whitespace-nowrap px-4 py-3.5 text-right" @click.stop>
                <div class="inline-flex items-center gap-1.5">
                  <Button
                    variant="outline"
                    size="sm"
                    data-artifact-preview-btn
                    @click="openArtifactModal(item)"
                  >
                    <Eye class="size-3.5" />{{ t('artifactsUi.clickToView') }}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    data-artifact-download-btn
                    :disabled="downloadingId === item.id"
                    @click="downloadArtifact(item)"
                  >
                    <Download class="size-3.5" />{{ t('jobUi.download') }}
                  </Button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <div v-else class="studio-panel p-10 text-center text-sm text-muted-foreground" data-artifact-empty>
      <FolderOpen class="mx-auto mb-3 size-8 opacity-60" />
      <p class="font-medium text-foreground">{{ t('artifactsUi.empty') }}</p>
      <p class="mx-auto mt-1 max-w-lg text-xs text-muted-foreground">{{ t('artifactsUi.emptyHint') }}</p>
    </div>

    <div class="flex justify-end gap-2">
      <Button variant="outline" :disabled="page <= 1 || loading" @click="load(page - 1)">
        {{ t('jobUi.previous') }}
      </Button>
      <Button variant="outline" :disabled="!hasMore || loading" @click="load(page + 1)">
        {{ t('jobUi.next') }}
      </Button>
    </div>

    <ArtifactPreviewDialog
      v-if="selectedArtifact"
      :key="selectedArtifact.id"
      :job-id="selectedArtifact.job_id"
      :artifact="selectedArtifact"
      :show-nav="items.length > 1"
      :has-prev="selectedIndex > 0"
      :has-next="selectedIndex >= 0 && selectedIndex < items.length - 1"
      @close="selectedArtifact = null"
      @download="selectedArtifact && downloadArtifact(selectedArtifact)"
      @prev="selectAdjacent(-1)"
      @next="selectAdjacent(1)"
      @open-job="openJob"
    />
  </section>
</template>

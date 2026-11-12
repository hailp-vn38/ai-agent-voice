<script setup lang="ts">
import {
  Activity, ArrowRight, Ban, Bot, Check, ChevronLeft, ChevronRight, CircleCheck, CircleDot, CircleStop, CircleX,
  ClipboardList, Clock, CornerDownLeft, FileAudio, FolderOpen, Hash, Hourglass, Library, ListChecks, LoaderCircle,
  Paperclip, Plus, RefreshCw, Repeat, RotateCcw, Search, Settings2, ShieldCheck, Sparkles, Timer, Trash2, Upload,
  Workflow, Wrench, X,
} from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref, useId, watch, type Component } from 'vue'
import { useRouter } from 'vue-router'
import { agentJobsApi } from '@/api/agent-jobs'
import { formatApiError, isApiError } from '@/api/errors'
import type { AdminJob, JobAttachmentUpload, JobStatus, RecipeOption, TaskRecipeSummary } from '@/api/types/agent-jobs'
import BaseModal from '@/components/admin/BaseModal.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import HeroScene from '@/components/admin/HeroScene.vue'
import RecipeParamsForm from '@/components/jobs/RecipeParamsForm.vue'
import { recipeFieldForValidationPath, recipeParamDefaults, validateRecipeParams } from '@/domain/jobs/recipe-form'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useErrorToast, useSuccessToast } from '@/composables/useNotify'

interface PendingWavAttachment extends JobAttachmentUpload {
  id: string
  sizeBytes: number
  lastModified: number
  previewUrl: string
}

const MAX_WAV_ATTACHMENTS = 4
const MAX_WAV_BYTES = 25 * 1024 * 1024

const router = useRouter()
const { t, formatRelative } = useI18n()
const createFormId = useId()
const jobs = ref<AdminJob[]>([])
const search = ref('')
const filter = ref('all')
const page = ref(1)
const total = ref(0)
const hasMore = ref(false)
const loading = ref(true)
const busy = ref(false)
const error = ref('')
const notice = ref('')
useSuccessToast(() => notice.value, { onDismiss: () => (notice.value = '') })
const backendPending = ref(false)
const createOpen = ref(false)
useErrorToast(() => (createOpen.value ? '' : error.value), { title: () => (backendPending.value ? t('jobUi.backendPending') : t('jobUi.loadFailed')), retry: () => load() })
const batchDeleteOpen = ref(false)
const selectedIds = ref<string[]>([])
const goal = ref('')
const recipes = ref<TaskRecipeSummary[]>([])
const recipesLoaded = ref(false)
const recipesLoading = ref(false)
const recipeCatalogError = ref('')
const recipeSearch = ref('')
const activeRecipe = ref<TaskRecipeSummary | null>(null)
const recipeParams = ref<Record<string, unknown>>({})
const recipeOptions = ref<Record<string, RecipeOption[]>>({})
const recipeFieldErrors = ref<Record<string, string>>({})
const recipeGlobalError = ref('')
const wavAttachments = ref<PendingWavAttachment[]>([])
const wavDragActive = ref(false)
const wavInputRef = ref<HTMLInputElement | null>(null)
const statusOptions: JobStatus[] = ['queued', 'planning', 'running', 'waiting', 'cancelling', 'succeeded', 'partial_success', 'failed', 'cancelled', 'stopped']
let controller: AbortController | undefined
let creation = { sig: '', key: '' }

const ACTIVE_STATUSES: JobStatus[] = ['queued', 'planning', 'running', 'waiting', 'cancelling']
const GOAL_MAX = 3000
const loadedAt = ref(Date.now())

/** One visual language per Job status, shared by rows, filter chips and the status strip. */
const statusMeta: Record<JobStatus, { icon: Component; tone: string; dot: string }> = {
  queued: { icon: Clock, tone: 'bg-muted text-muted-foreground', dot: 'bg-muted-foreground/60' },
  planning: { icon: Sparkles, tone: 'bg-studio-violet/10 text-studio-violet', dot: 'bg-studio-violet' },
  running: { icon: LoaderCircle, tone: 'bg-studio-cyan/10 text-studio-cyan', dot: 'bg-studio-cyan' },
  waiting: { icon: Hourglass, tone: 'bg-warning/10 text-warning', dot: 'bg-warning' },
  cancelling: { icon: Ban, tone: 'bg-warning/10 text-warning', dot: 'bg-warning/60' },
  succeeded: { icon: CircleCheck, tone: 'bg-success/10 text-success', dot: 'bg-success' },
  partial_success: { icon: CircleDot, tone: 'bg-success/10 text-success', dot: 'bg-success/55' },
  failed: { icon: CircleX, tone: 'bg-danger/10 text-danger', dot: 'bg-danger' },
  cancelled: { icon: Ban, tone: 'bg-muted text-muted-foreground', dot: 'bg-muted-foreground/40' },
  stopped: { icon: CircleStop, tone: 'bg-muted text-muted-foreground', dot: 'bg-muted-foreground/25' },
}
const isActive = (status: JobStatus) => ACTIVE_STATUSES.includes(status)

const activeCount = computed(() => jobs.value.filter(job => isActive(job.status)).length)
const completedCount = computed(() => jobs.value.filter(job => ['succeeded', 'partial_success'].includes(job.status)).length)
const failedCount = computed(() => jobs.value.filter(job => job.status === 'failed').length)
const successRate = computed(() => {
  const settled = completedCount.value + failedCount.value
  return settled ? Math.round((completedCount.value / settled) * 100) : null
})
const statusCounts = computed(() => {
  const counts = {} as Record<JobStatus, number>
  for (const job of jobs.value) counts[job.status] = (counts[job.status] ?? 0) + 1
  return counts
})
const statusMix = computed(() => statusOptions
  .map(status => ({ status, count: statusCounts.value[status] ?? 0 }))
  .filter(item => item.count > 0))
const hasFilters = computed(() => filter.value !== 'all' || search.value.trim().length > 0)
const pageRange = computed(() => {
  if (!jobs.value.length) return ''
  const from = (page.value - 1) * 30 + 1
  return t('jobUi.pageRange', { from, to: from + jobs.value.length - 1, total: total.value })
})
const allSelected = computed(() => jobs.value.length > 0 && jobs.value.every(job => selectedIds.value.includes(job.id)))
const visibleRecipes = computed(() => {
  const query = recipeSearch.value.trim().toLocaleLowerCase()
  return recipes.value.filter(recipe => !query || `${recipe.title} ${recipe.description}`.toLocaleLowerCase().includes(query))
})
const canSubmit = computed(() => !busy.value && (activeRecipe.value
  ? Object.keys(validateRecipeParams(activeRecipe.value.params_schema, recipeParams.value)).length === 0
  : goal.value.trim().length >= 5))
const goalExamples = computed(() => [t('jobUi.example1'), t('jobUi.example2'), t('jobUi.example3')])
const label = (status: JobStatus) => t(('jobUi.jobStatus.' + status) as 'jobUi.jobStatus.queued')
/** The API sends timestamps as ISO strings or as epoch seconds. */
function toMillis(value: string | number | null | undefined) {
  if (value == null || value === '') return Number.NaN
  const numeric = Number(value)
  if (Number.isFinite(numeric)) return numeric < 1e12 ? numeric * 1000 : numeric
  return Date.parse(String(value))
}
function date(value: string) {
  const stamp = toMillis(value)
  return Number.isFinite(stamp) ? new Date(stamp).toLocaleString() : value
}
function relative(value: string) {
  const stamp = toMillis(value)
  return Number.isFinite(stamp) ? formatRelative(new Date(stamp).toISOString()) : value
}
function formatDuration(ms: number) {
  const seconds = Math.max(0, Math.round(ms / 1000))
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`
}
/** Run time of a finished Job, or elapsed time (as of the last refresh) of a live one. */
function duration(job: AdminJob) {
  const start = toMillis(job.started_at)
  if (!Number.isFinite(start)) return ''
  const end = job.finished_at ? toMillis(job.finished_at) : isActive(job.status) ? loadedAt.value : Number.NaN
  return Number.isFinite(end) ? formatDuration(end - start) : ''
}
function recipePercent(job: AdminJob) {
  const steps = job.recipe?.outline?.length ?? 0
  if (!steps) return null
  return Math.round(Math.min(1, (job.recipe?.progress?.completed_steps?.length ?? 0) / steps) * 100)
}
function setFilter(value: string) {
  if (filter.value === value) return
  filter.value = value
  void load()
}
function clearFilters() {
  filter.value = 'all'
  search.value = ''
  void load()
}
async function load(nextPage = 1) {
  controller?.abort()
  const pending = new AbortController()
  controller = pending
  loading.value = true
  error.value = ''
  try {
    const result = await agentJobsApi.listJobs({
      page: nextPage,
      status: filter.value === 'all' ? undefined : filter.value,
      q: search.value.trim() || undefined,
    }, pending.signal)
    if (pending.signal.aborted) return
    jobs.value = result.items
    selectedIds.value = selectedIds.value.filter(id => result.items.some(job => job.id === id))
    total.value = result.total ?? result.items.length
    page.value = result.page
    hasMore.value = (result.page * result.page_size) < (result.total ?? (result.page * result.page_size + (result.items.length === result.page_size ? 1 : 0)))
    loadedAt.value = Date.now()
    backendPending.value = false
  } catch (cause) {
    if (pending.signal.aborted) return
    backendPending.value = isApiError(cause) && cause.status === 404
    error.value = formatApiError(cause)
  } finally {
    if (controller === pending) loading.value = false
  }
}
function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`
}

function clearWavAttachments() {
  for (const item of wavAttachments.value) {
    if (item.previewUrl) URL.revokeObjectURL(item.previewUrl)
  }
  wavAttachments.value = []
}

async function loadRecipeCatalog() {
  if (recipesLoaded.value || recipesLoading.value) return
  recipesLoading.value = true
  recipeCatalogError.value = ''
  try {
    const catalog = await agentJobsApi.listRecipes()
    recipes.value = catalog.items
    recipesLoaded.value = true
    const sources = new Set(catalog.items.flatMap(recipe => Object.values(recipe.params_schema.properties)
      .map(field => field['x-options-source']).filter((source): source is string => Boolean(source))))
    for (const source of sources) {
      if (!/wiki.*domain|domain.*wiki/i.test(source)) continue
      try { recipeOptions.value[source] = await agentJobsApi.wikiDomains() }
      catch { recipeCatalogError.value = t('jobUi.recipeOptionsFailed') }
    }
  } catch (cause) {
    recipeCatalogError.value = formatApiError(cause)
  } finally {
    recipesLoading.value = false
  }
}

function selectRecipe(recipe: TaskRecipeSummary | null) {
  activeRecipe.value = recipe
  recipeFieldErrors.value = {}
  recipeGlobalError.value = ''
  error.value = ''
  if (recipe) recipeParams.value = recipeParamDefaults(recipe.params_schema)
}

function removeWavAttachment(id: string) {
  const idx = wavAttachments.value.findIndex(item => item.id === id)
  if (idx >= 0) {
    const [removed] = wavAttachments.value.splice(idx, 1)
    if (removed?.previewUrl) URL.revokeObjectURL(removed.previewUrl)
  }
}

function readFileAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const result = typeof reader.result === 'string' ? reader.result : ''
      const commaIdx = result.indexOf(',')
      resolve(commaIdx >= 0 ? result.slice(commaIdx + 1) : result)
    }
    reader.onerror = () => reject(reader.error ?? new Error('Failed to read file'))
    reader.readAsDataURL(file)
  })
}

function hasRiffMagic(base64Payload: string): boolean {
  try {
    const head = atob(base64Payload.slice(0, 8))
    return head.startsWith('RIFF')
  } catch {
    return false
  }
}

function hasMp4FtypMagic(base64Payload: string): boolean {
  try {
    const head = atob(base64Payload.slice(0, 16))
    return head.length >= 8 && head.slice(4, 8) === 'ftyp'
  } catch {
    return false
  }
}

async function addWavFiles(fileList: FileList | File[]) {
  error.value = ''
  const files = Array.from(fileList)
  if (!files.length) return
  if (wavAttachments.value.length + files.length > MAX_WAV_ATTACHMENTS) {
    error.value = t('jobUi.wavTooMany')
    return
  }
  for (const file of files) {
    const lowerName = file.name.trim().toLowerCase()
    const isWav = lowerName.endsWith('.wav')
    const isMp4 = lowerName.endsWith('.mp4')
    const rawMime = (file.type || '').toLowerCase()
    const validWavMime = !rawMime || rawMime === 'audio/wav' || rawMime === 'audio/x-wav' || rawMime === 'audio/wave' || rawMime === 'audio/vnd.wave'
    const validMp4Mime = !rawMime || rawMime === 'video/mp4'
    if ((!isWav || !validWavMime) && (!isMp4 || !validMp4Mime)) {
      error.value = t('jobUi.wavInvalidFormat')
      return
    }
    if (file.size <= 0 || file.size > MAX_WAV_BYTES) {
      error.value = t('jobUi.wavTooLarge')
      return
    }
  }
  const staged: PendingWavAttachment[] = []
  for (const file of files) {
    const lowerName = file.name.trim().toLowerCase()
    const isMp4 = lowerName.endsWith('.mp4')
    const data_base64 = await readFileAsBase64(file)
    if (isMp4 ? !hasMp4FtypMagic(data_base64) : !hasRiffMagic(data_base64)) {
      error.value = t('jobUi.wavInvalidFormat')
      return
    }
    const previewUrl = typeof URL.createObjectURL === 'function' ? URL.createObjectURL(file) : ''
    staged.push({
      id: crypto.randomUUID(),
      name: file.name.trim(),
      mime_type: isMp4 ? 'video/mp4' : 'audio/wav',
      data_base64,
      sizeBytes: file.size,
      lastModified: file.lastModified || 0,
      previewUrl,
    })
  }
  wavAttachments.value.push(...staged)
  if (!goal.value.trim() && wavAttachments.value.length > 0) {
    goal.value = t('jobUi.wavDefaultGoal')
  }
}

function onWavInputChange(event: Event) {
  const target = event.target as HTMLInputElement | null
  if (target?.files) {
    void addWavFiles(target.files)
    target.value = ''
  }
}

function onWavDrop(event: DragEvent) {
  wavDragActive.value = false
  if (event.dataTransfer?.files) {
    void addWavFiles(event.dataTransfer.files)
  }
}

watch(createOpen, isOpen => {
  if (!isOpen && !busy.value) {
    clearWavAttachments()
  } else if (isOpen) {
    void loadRecipeCatalog()
  }
})

async function create() {
  if (busy.value) return
  if (activeRecipe.value) {
    const validation = validateRecipeParams(activeRecipe.value.params_schema, recipeParams.value)
    recipeFieldErrors.value = validation
    if (Object.keys(validation).length) return
  } else if (goal.value.trim().length < 5) return
  busy.value = true
  error.value = ''
  recipeGlobalError.value = ''
  notice.value = ''
  try {
    const trimmedGoal = goal.value.trim()
    const attSig = wavAttachments.value.map(a => `${a.name}:${a.sizeBytes}:${a.lastModified}`).join('|')
    const nextSig = activeRecipe.value
      ? `recipe:${activeRecipe.value.id}:${JSON.stringify(recipeParams.value)}`
      : `${trimmedGoal}::${attSig}`
    if (creation.sig !== nextSig) creation = { sig: nextSig, key: crypto.randomUUID() }
    const job = activeRecipe.value
      ? await agentJobsApi.createRecipeJob(activeRecipe.value.id, recipeParams.value, creation.key)
      : await agentJobsApi.createJob(trimmedGoal, creation.key, wavAttachments.value.map(a => ({
        name: a.name,
        mime_type: a.mime_type,
        data_base64: a.data_base64,
      } satisfies JobAttachmentUpload)))
    creation = { sig: '', key: '' }
    clearWavAttachments()
    createOpen.value = false
    goal.value = ''
    activeRecipe.value = null
    recipeParams.value = {}
    await router.push({ name: 'job-detail', params: { jobId: job.id } })
  } catch (cause) {
    if (activeRecipe.value && isApiError(cause) && cause.validationPath) {
      const field = recipeFieldForValidationPath(activeRecipe.value.params_schema, cause.validationPath)
      if (field) recipeFieldErrors.value = { ...recipeFieldErrors.value, [field]: formatApiError(cause) }
      else recipeGlobalError.value = formatApiError(cause)
    } else if (activeRecipe.value) recipeGlobalError.value = formatApiError(cause)
    else error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}
function openDetail(id: string) {
  void router.push({ name: 'job-detail', params: { jobId: id } })
}
function toggleSelectAll(event: Event) {
  const checked = (event.target as HTMLInputElement).checked
  selectedIds.value = checked ? jobs.value.map(job => job.id) : []
}
function toggleSelectJob(id: string, event: Event) {
  const checked = (event.target as HTMLInputElement).checked
  if (checked) {
    if (!selectedIds.value.includes(id)) selectedIds.value.push(id)
  } else {
    selectedIds.value = selectedIds.value.filter(item => item !== id)
  }
}
async function deleteSelectedJobs() {
  if (!selectedIds.value.length || busy.value) return
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    const ids = [...selectedIds.value]
    const res = await agentJobsApi.deleteJobs(ids)
    selectedIds.value = []
    notice.value = t('jobUi.jobsDeleted', { count: res.deleted })
    await load(page.value)
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}
onMounted(() => { void load() })
onBeforeUnmount(() => {
  controller?.abort()
  clearWavAttachments()
})
</script>

<template>
  <section class="space-y-5">
    <!-- Hero: title, actions and the live pulse of this page -->
    <header class="jobs-hero relative isolate overflow-hidden rounded-3xl p-4 text-white shadow-lg sm:p-5" aria-labelledby="jobs-title">
      <div class="relative z-10 flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div class="min-w-0 max-w-2xl">
          <p class="text-xs font-semibold uppercase tracking-[0.2em] text-white/70">{{ t('jobUi.heroEyebrow') }}</p>
          <h1 id="jobs-title" class="mt-1.5 text-3xl font-semibold tracking-tight">{{ t('jobUi.jobs') }}</h1>
          <p class="mt-1.5 text-sm text-white/80">{{ t('jobUi.jobsDescription') }}</p>
          <div class="mt-2.5 inline-flex items-center gap-2 rounded-full bg-white/12 px-3 py-1 text-xs font-medium backdrop-blur" role="status">
            <span class="relative flex size-2.5">
              <span v-if="activeCount" class="absolute inline-flex size-full animate-ping rounded-full bg-emerald-300 opacity-75" />
              <span class="relative inline-flex size-2.5 rounded-full" :class="activeCount ? 'bg-emerald-300' : 'bg-white/50'" />
            </span>
            {{ activeCount ? t('jobUi.liveActive', { count: activeCount }) : t('jobUi.liveIdle') }}
          </div>
        </div>

        <div class="flex flex-col items-start gap-3 lg:items-end">
          <div class="flex flex-wrap items-center gap-2">
            <button type="button" class="jobs-hero-primary" @click="createOpen = true"><Plus class="size-4" aria-hidden="true" />{{ t('jobUi.newJob') }}</button>
            <button type="button" class="jobs-hero-ghost" :disabled="loading" @click="load()"><RefreshCw class="size-4" :class="{ 'animate-spin': loading }" aria-hidden="true" />{{ t('jobUi.refresh') }}</button>
            <button type="button" class="jobs-hero-ghost" @click="router.push({ name: 'artifacts' })"><FolderOpen class="size-4" aria-hidden="true" />{{ t('nav.artifacts') }}</button>
            <button type="button" class="jobs-hero-ghost !px-2.5" :aria-label="t('jobUi.background')" :title="t('jobUi.background')" @click="router.push({ name: 'jobs-settings' })"><Settings2 class="size-4" aria-hidden="true" /></button>
          </div>
          <p class="text-xs text-white/70">{{ t('jobUi.lastUpdated', { time: new Date(loadedAt).toLocaleTimeString() }) }}</p>
        </div>
      </div>

      <HeroScene theme="jobs" />
      <dl class="relative z-10 mt-4 grid grid-cols-2 gap-2 sm:gap-3 xl:grid-cols-4">
        <div class="jobs-hero-stat">
          <dt><ClipboardList class="size-3.5" aria-hidden="true" />{{ t('jobUi.totalFiltered') }}</dt>
          <dd>{{ total }}</dd>
        </div>
        <div class="jobs-hero-stat">
          <dt><Activity class="size-3.5" aria-hidden="true" />{{ t('jobUi.activeJobs') }}</dt>
          <dd>{{ activeCount }}</dd>
        </div>
        <div class="jobs-hero-stat">
          <dt><CircleCheck class="size-3.5" aria-hidden="true" />{{ t('jobUi.doneJobs') }}</dt>
          <dd>
            {{ completedCount }}
            <span v-if="successRate !== null" class="ml-1 text-xs font-medium text-white/70">{{ successRate }}% {{ t('jobUi.successRate') }}</span>
          </dd>
        </div>
        <div class="jobs-hero-stat">
          <dt><CircleX class="size-3.5" aria-hidden="true" />{{ t('jobUi.failedJobs') }}</dt>
          <dd>{{ failedCount }}</dd>
        </div>
      </dl>
    </header>

    <!-- Search + status chips -->
    <div class="studio-panel space-y-3 p-3 sm:p-4">
      <form class="relative" role="search" @submit.prevent="load()">
        <Search class="pointer-events-none absolute left-3 top-3 size-4 text-muted-foreground" aria-hidden="true" />
        <input v-model="search" class="admin-input pl-9" type="search" :placeholder="t('jobUi.searchJobs')" :aria-label="t('jobUi.searchJobs')" />
      </form>
      <div class="flex flex-wrap items-center gap-1.5" role="group" :aria-label="t('jobUi.filterStatus')">
        <button type="button" class="jobs-chip" :class="{ 'jobs-chip-active': filter === 'all' }" :aria-pressed="filter === 'all'" @click="setFilter('all')">
          {{ t('jobUi.allStatus') }}
        </button>
        <button
          v-for="status in statusOptions"
          :key="status"
          type="button"
          class="jobs-chip"
          :class="{ 'jobs-chip-active': filter === status }"
          :aria-pressed="filter === status"
          :data-status-chip="status"
          @click="setFilter(status)"
        >
          <span class="size-2 rounded-full" :class="statusMeta[status].dot" aria-hidden="true" />
          {{ label(status) }}
          <span v-if="statusCounts[status]" class="rounded-full bg-muted px-1.5 text-[10px] font-semibold tabular-nums text-muted-foreground">{{ statusCounts[status] }}</span>
        </button>
        <button v-if="hasFilters" type="button" class="ml-auto inline-flex items-center gap-1 rounded-full px-2.5 py-1 text-xs font-medium text-muted-foreground transition hover:bg-accent hover:text-foreground" @click="clearFilters">
          <X class="size-3.5" aria-hidden="true" />{{ t('jobUi.clearFilters') }}
        </button>
      </div>
    </div>

    <!-- List -->
    <div v-if="loading && !jobs.length" class="studio-panel divide-y overflow-hidden" role="status" :aria-label="t('common.loading')">
      <div v-for="row in 6" :key="row" class="flex items-center gap-4 p-4">
        <div class="card-shimmer size-9 shrink-0 rounded-xl bg-muted/60" />
        <div class="min-w-0 flex-1 space-y-2">
          <div class="card-shimmer h-3.5 rounded bg-muted/60" :style="{ width: `${70 - row * 6}%` }" />
          <div class="card-shimmer h-3 w-1/4 rounded bg-muted/40" />
        </div>
        <div class="card-shimmer hidden h-6 w-24 rounded-full bg-muted/60 sm:block" />
      </div>
    </div>
    <div v-else-if="jobs.length" class="studio-panel overflow-hidden transition-opacity" :class="{ 'opacity-60': loading }" :aria-busy="loading">
      <div class="flex h-1.5 bg-muted" role="img" :aria-label="t('jobUi.statusMix')">
        <div
          v-for="item in statusMix"
          :key="item.status"
          class="transition-all duration-500"
          :class="statusMeta[item.status].dot"
          :style="{ width: `${(item.count / jobs.length) * 100}%` }"
          :title="`${label(item.status)} · ${item.count}`"
        />
      </div>
      <div class="overflow-x-auto">
        <table class="w-full text-left text-sm">
          <thead class="border-b bg-muted/40 text-xs text-muted-foreground">
            <tr>
              <th class="w-10 py-3 pl-4 pr-1">
                <input
                  type="checkbox"
                  class="size-4 cursor-pointer rounded border-border accent-studio-violet"
                  :checked="allSelected"
                  :aria-label="t('jobUi.selectAllJobs')"
                  @change="toggleSelectAll"
                />
              </th>
              <th class="px-3 py-3 font-medium">{{ t('jobUi.goal') }}</th>
              <th class="px-3 py-3 font-medium">{{ t('jobUi.requester') }}</th>
              <th class="px-3 py-3 font-medium">{{ t('jobUi.status') }}</th>
              <th class="px-3 py-3 font-medium">{{ t('jobUi.createdAt') }}</th>
              <th class="px-3 py-3 font-medium"><span class="sr-only">{{ t('jobUi.details') }}</span></th>
            </tr>
          </thead>
          <tbody class="divide-y">
            <tr
              v-for="job in jobs"
              :key="job.id"
              class="group transition-colors hover:bg-muted/30"
              :class="[selectedIds.includes(job.id) ? 'bg-studio-violet/5' : '', isActive(job.status) ? 'jobs-row-live' : '']"
            >
              <td class="w-10 py-4 pl-4 pr-1 align-top">
                <input
                  type="checkbox"
                  class="mt-2.5 size-4 cursor-pointer rounded border-border accent-studio-violet"
                  :checked="selectedIds.includes(job.id)"
                  :aria-label="t('jobUi.selectJob', { id: job.id })"
                  @change="toggleSelectJob(job.id, $event)"
                />
              </td>
              <td class="max-w-md px-3 py-4">
                <div class="flex items-start gap-3">
                  <span class="flex size-9 shrink-0 items-center justify-center rounded-xl" :class="statusMeta[job.status].tone" aria-hidden="true">
                    <component :is="statusMeta[job.status].icon" class="size-4" :class="{ 'animate-spin': job.status === 'running' }" />
                  </span>
                  <div class="min-w-0 flex-1">
                    <button class="block w-full cursor-pointer truncate text-left font-medium transition-colors hover:text-studio-violet" type="button" :title="job.goal" @click="openDetail(job.id)">{{ job.title || job.goal }}</button>
                    <p v-if="job.title && job.title !== job.goal" class="mt-0.5 truncate text-xs text-muted-foreground">{{ job.goal }}</p>
                    <p v-if="job.status === 'failed' && job.error_message" class="mt-0.5 truncate text-xs text-danger" :title="job.error_message">{{ job.error_message }}</p>
                    <div class="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
                      <span v-if="job.recipe" class="inline-flex min-w-0 items-center gap-1 rounded-md bg-studio-violet/10 px-1.5 py-0.5 font-medium text-studio-violet">
                        <Workflow class="size-3 shrink-0" aria-hidden="true" /><span class="truncate">recipe · {{ job.recipe.title }} v{{ job.recipe.version }}</span>
                      </span>
                      <span v-if="job.execution_mode === 'knowledge_ingest_v3'" class="inline-flex items-center gap-1 rounded-md bg-studio-cyan/10 px-1.5 py-0.5 font-medium text-studio-cyan">
                        <Library class="size-3" aria-hidden="true" />{{ t('jobUi.knowledge') }}
                      </span>
                      <span v-if="job.retry_of_job_id" class="inline-flex items-center gap-1" :title="`${t('jobUi.retryOf')} ${job.retry_of_job_id}`">
                        <RotateCcw class="size-3" aria-hidden="true" />{{ t('jobUi.retryBadge') }}
                      </span>
                      <span v-if="job.artifacts?.length" class="inline-flex items-center gap-1">
                        <Paperclip class="size-3" aria-hidden="true" />{{ t('jobUi.artifactCount', { count: job.artifacts.length }) }}
                      </span>
                      <span class="inline-flex min-w-0 items-center gap-0.5 font-mono" :title="job.id">
                        <Hash class="size-3 shrink-0" aria-hidden="true" /><span class="truncate">{{ job.id }}</span>
                      </span>
                    </div>
                    <div v-if="recipePercent(job) !== null" class="mt-2 flex items-center gap-2" :title="t('jobUi.recipeProgress')">
                      <div class="h-1 flex-1 overflow-hidden rounded-full bg-muted">
                        <div class="h-full rounded-full bg-studio-violet transition-all duration-500" :style="{ width: `${recipePercent(job)}%` }" />
                      </div>
                      <span class="text-[10px] font-medium tabular-nums text-muted-foreground">{{ recipePercent(job) }}%</span>
                    </div>
                  </div>
                </div>
              </td>
              <td class="max-w-48 px-3 py-4 align-top">
                <span class="inline-flex max-w-full items-center gap-1.5 rounded-full bg-surface px-2.5 py-1 text-xs text-muted-foreground" :title="job.requester_scope">
                  <component :is="job.requester_scope.startsWith('admin') ? ShieldCheck : Bot" class="size-3.5 shrink-0" aria-hidden="true" />
                  <span class="truncate">{{ job.requester_scope }}</span>
                </span>
              </td>
              <td class="px-3 py-4 align-top">
                <span class="inline-flex items-center gap-1.5 whitespace-nowrap rounded-full px-2.5 py-1 text-xs font-semibold" :class="statusMeta[job.status].tone">
                  <span class="relative flex size-1.5" aria-hidden="true">
                    <span v-if="isActive(job.status)" class="absolute inline-flex size-full animate-ping rounded-full opacity-75" :class="statusMeta[job.status].dot" />
                    <span class="relative inline-flex size-1.5 rounded-full" :class="statusMeta[job.status].dot" />
                  </span>
                  {{ label(job.status) }}
                </span>
              </td>
              <td class="whitespace-nowrap px-3 py-4 align-top text-xs text-muted-foreground">
                <p class="text-foreground" :title="date(job.created_at)">{{ relative(job.created_at) }}</p>
                <p v-if="duration(job)" class="mt-1 inline-flex items-center gap-1 tabular-nums"><Timer class="size-3" aria-hidden="true" />{{ duration(job) }}</p>
              </td>
              <td class="px-3 py-4 text-right align-top">
                <Button class="cursor-pointer transition-transform group-hover:translate-x-0.5" variant="ghost" size="icon" :aria-label="t('jobUi.details')" @click="openDetail(job.id)"><ArrowRight class="size-4" /></Button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
    <div v-else class="studio-panel flex flex-col items-center px-6 py-14 text-center">
      <span class="flex size-14 items-center justify-center rounded-2xl bg-primary/10 text-studio-violet"><ClipboardList class="size-7" aria-hidden="true" /></span>
      <p class="mt-4 text-base font-semibold">{{ hasFilters ? t('jobUi.noMatches') : t('jobUi.noJobs') }}</p>
      <p v-if="!hasFilters" class="mt-1 max-w-sm text-sm text-muted-foreground">{{ t('jobUi.emptyHint') }}</p>
      <Button v-if="hasFilters" class="mt-5" variant="outline" @click="clearFilters"><X class="size-4" />{{ t('jobUi.clearFilters') }}</Button>
      <Button v-else class="mt-5" @click="createOpen = true"><Plus class="size-4" />{{ t('jobUi.newJob') }}</Button>
    </div>

    <div class="flex flex-wrap items-center justify-between gap-2">
      <p class="text-xs tabular-nums text-muted-foreground">
        {{ t('jobUi.pageLabel', { page }) }}<template v-if="pageRange"> · {{ pageRange }}</template>
      </p>
      <div class="flex gap-2">
        <Button variant="outline" :disabled="page <= 1 || loading" @click="load(page - 1)"><ChevronLeft class="size-4" />{{ t('jobUi.previous') }}</Button>
        <Button variant="outline" :disabled="!hasMore || loading" @click="load(page + 1)">{{ t('jobUi.next') }}<ChevronRight class="size-4" /></Button>
      </div>
    </div>

    <!-- Floating bulk actions -->
    <Transition name="jobs-bulk">
      <div v-if="selectedIds.length > 0" class="sticky bottom-4 z-20 mx-auto flex w-fit max-w-full items-center gap-1 rounded-full border bg-background/95 py-1.5 pl-4 pr-1.5 shadow-xl backdrop-blur">
        <span class="mr-2 text-sm font-medium tabular-nums">{{ t('jobUi.selectedCount', { count: selectedIds.length }) }}</span>
        <Button variant="ghost" size="sm" class="rounded-full" @click="selectedIds = []">{{ t('jobUi.clearAll') }}</Button>
        <Button
          size="sm"
          class="rounded-full bg-danger text-white hover:bg-danger/90"
          data-testid="batch-delete-jobs"
          :disabled="busy || loading"
          @click="batchDeleteOpen = true"
        >
          <Trash2 class="size-3.5" />{{ t('jobUi.deleteSelectedJobs', { count: selectedIds.length }) }}
        </Button>
      </div>
    </Transition>

    <!-- New Job composer -->
    <BaseModal v-model="createOpen" :title="t('jobUi.newJob')" :description="t('jobUi.createHint')" width-class="max-w-5xl">
      <form :id="createFormId" class="grid gap-5 lg:grid-cols-[17rem_minmax(0,1fr)]" @submit.prevent="create">
        <!-- Left rail: how to start -->
        <aside class="space-y-3 lg:sticky lg:top-0 lg:self-start">
          <p class="text-xs font-semibold uppercase tracking-wider text-muted-foreground">{{ t('jobUi.startFrom') }}</p>
          <button
            type="button"
            class="jobs-pick"
            :class="{ 'jobs-pick-active': activeRecipe === null }"
            :aria-pressed="activeRecipe === null"
            data-testid="free-job-card"
            @click="selectRecipe(null)"
          >
            <span class="jobs-pick-icon jobs-pick-icon-free"><Sparkles class="size-4" aria-hidden="true" /></span>
            <span class="min-w-0 flex-1">
              <span class="block font-medium">{{ t('jobUi.freeRecipe') }}</span>
              <span class="mt-0.5 block text-xs text-muted-foreground">{{ t('jobUi.freeRecipeDescription') }}</span>
            </span>
            <Check v-if="activeRecipe === null" class="size-4 shrink-0 text-studio-violet" aria-hidden="true" />
          </button>

          <div class="flex items-center gap-2 pt-1">
            <p class="text-xs font-semibold uppercase tracking-wider text-muted-foreground">{{ t('jobUi.recipesGroup') }}</p>
            <span v-if="recipesLoaded" class="rounded-full bg-muted px-1.5 text-[10px] font-semibold tabular-nums text-muted-foreground">{{ recipes.length }}</span>
          </div>
          <div class="relative">
            <Search class="pointer-events-none absolute left-3 top-3 size-4 text-muted-foreground" aria-hidden="true" />
            <input v-model="recipeSearch" class="admin-input pl-9" type="search" :placeholder="t('jobUi.searchRecipes')" :aria-label="t('jobUi.searchRecipes')" />
          </div>
          <div class="space-y-2 lg:max-h-[42vh] lg:overflow-y-auto lg:pr-1">
            <button
              v-for="recipe in visibleRecipes"
              :key="recipe.id"
              type="button"
              class="jobs-pick"
              :class="{ 'jobs-pick-active': activeRecipe?.id === recipe.id }"
              :aria-pressed="activeRecipe?.id === recipe.id"
              :data-recipe-id="recipe.id"
              @click="selectRecipe(recipe)"
            >
              <span class="jobs-pick-icon"><Workflow class="size-4" aria-hidden="true" /></span>
              <span class="min-w-0 flex-1">
                <span class="flex items-center justify-between gap-2 font-medium"><span class="truncate">{{ recipe.title }}</span><span class="shrink-0 text-xs font-normal text-muted-foreground">v{{ recipe.version }}</span></span>
                <span class="mt-0.5 line-clamp-2 block text-xs text-muted-foreground">{{ recipe.description }}</span>
                <span class="mt-1.5 inline-flex items-center gap-1 text-[11px] text-muted-foreground"><ListChecks class="size-3" aria-hidden="true" />{{ t('jobUi.stepCount', { count: recipe.outline.length }) }}</span>
              </span>
            </button>
            <div v-if="recipesLoading" class="space-y-2" role="status" :aria-label="t('common.loading')">
              <div v-for="row in 2" :key="row" class="card-shimmer h-16 rounded-xl bg-muted/50" />
            </div>
            <p v-else-if="recipesLoaded && !visibleRecipes.length" class="rounded-xl border border-dashed p-3 text-center text-xs text-muted-foreground">{{ t('jobUi.noRecipes') }}</p>
          </div>
          <p v-if="recipeCatalogError" role="status" class="text-xs text-muted-foreground">{{ recipeCatalogError }}</p>
        </aside>

        <!-- Right: configure -->
        <div class="min-w-0 space-y-4 lg:border-l lg:pl-5">
          <template v-if="activeRecipe">
            <div class="jobs-recipe-head flex items-start gap-3 rounded-xl border p-4">
              <span class="jobs-pick-icon"><Workflow class="size-4" aria-hidden="true" /></span>
              <div class="min-w-0 flex-1">
                <h3 class="font-semibold">{{ activeRecipe.title }}</h3>
                <p class="mt-1 text-xs text-muted-foreground">{{ activeRecipe.description }}</p>
              </div>
              <span class="shrink-0 rounded-full border bg-background px-2 py-0.5 text-xs font-semibold">v{{ activeRecipe.version }}</span>
            </div>

            <section v-if="Object.keys(activeRecipe.params_schema.properties).length" class="space-y-3">
              <h3 class="text-sm font-semibold">{{ t('jobUi.parameters') }}</h3>
              <RecipeParamsForm v-model="recipeParams" :schema="activeRecipe.params_schema" :options-by-source="recipeOptions" :field-errors="recipeFieldErrors" />
            </section>

            <section class="space-y-3" aria-label="Recipe outline">
              <h3 class="text-sm font-semibold">{{ t('jobUi.recipeSteps') }}</h3>
              <ol class="jobs-steps">
                <li v-for="(step, index) in activeRecipe.outline" :key="`${index}-${step.title}`" class="jobs-step">
                  <span class="jobs-step-node" :class="step.kind === 'tool' ? 'bg-studio-cyan/10 text-studio-cyan' : 'bg-studio-violet/10 text-studio-violet'" aria-hidden="true">
                    <component :is="step.kind === 'tool' ? Wrench : Bot" class="size-3.5" />
                  </span>
                  <div class="min-w-0 flex-1">
                    <p class="text-sm"><span class="mr-1.5 text-xs tabular-nums text-muted-foreground">{{ index + 1 }}.</span>{{ step.title }}</p>
                    <div class="mt-1 flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
                      <span class="rounded-md bg-muted px-1.5 py-0.5 font-medium">{{ step.kind === 'tool' ? t('jobUi.recipeTool') : t('jobUi.recipeAgent') }}</span>
                      <span v-if="step.repeat" class="inline-flex items-center gap-1 rounded-md bg-muted px-1.5 py-0.5"><Repeat class="size-3" aria-hidden="true" />{{ t('jobUi.recipeRepeats') }}</span>
                      <code v-for="name in [...(step.host_tools ?? []), ...(step.sub_agents ?? [])]" :key="name" class="rounded-md border px-1.5 py-0.5">{{ name }}</code>
                    </div>
                  </div>
                </li>
              </ol>
            </section>
            <p v-if="recipeGlobalError" role="alert" class="rounded-lg bg-danger/10 px-3 py-2 text-sm text-danger">{{ recipeGlobalError }}</p>
          </template>

          <template v-else>
            <div>
              <label :for="`${createFormId}-goal`" class="text-sm font-semibold">{{ t('jobUi.goal') }}</label>
              <p class="mt-1 text-xs text-muted-foreground">{{ t('jobUi.goalHint') }}</p>
            </div>
            <div class="jobs-composer">
              <textarea
                :id="`${createFormId}-goal`"
                v-model="goal"
                class="block min-h-36 w-full resize-y bg-transparent px-4 py-3 text-sm outline-none placeholder:text-muted-foreground"
                :maxlength="GOAL_MAX"
                required
                :placeholder="t('jobUi.goalPlaceholder')"
                @keydown.enter.meta.prevent="create"
                @keydown.enter.ctrl.prevent="create"
              />
              <div class="flex items-center justify-between gap-2 border-t px-3 py-2 text-xs text-muted-foreground">
                <span class="inline-flex items-center gap-1.5">
                  <kbd class="jobs-kbd">⌘</kbd><kbd class="jobs-kbd"><CornerDownLeft class="size-3" aria-hidden="true" /></kbd>{{ t('jobUi.submitShortcut') }}
                </span>
                <span class="tabular-nums" :class="{ 'text-warning': goal.length > GOAL_MAX * 0.9 }">{{ goal.length }}/{{ GOAL_MAX }}</span>
              </div>
            </div>
            <div v-if="!goal.trim()" class="flex flex-wrap items-center gap-1.5">
              <span class="text-xs text-muted-foreground">{{ t('jobUi.tryExample') }}</span>
              <button v-for="example in goalExamples" :key="example" type="button" class="jobs-chip max-w-full" :title="example" @click="goal = example">
                <Sparkles class="size-3 shrink-0 text-studio-violet" aria-hidden="true" /><span class="truncate">{{ example }}</span>
              </button>
            </div>

            <div class="space-y-2">
              <div class="flex items-center justify-between">
                <span class="text-sm font-semibold">{{ t('jobUi.wavAttachments') }}</span>
                <span class="rounded-full bg-muted px-2 py-0.5 text-xs tabular-nums text-muted-foreground">{{ wavAttachments.length }}/{{ MAX_WAV_ATTACHMENTS }}</span>
              </div>
              <input
                ref="wavInputRef"
                type="file"
                accept=".wav,.mp4,audio/wav,audio/x-wav,audio/wave,video/mp4"
                multiple
                class="sr-only"
                data-testid="job-wav-file-input"
                @change="onWavInputChange"
              />
              <div
                role="button"
                tabindex="0"
                data-testid="job-wav-dropzone"
                class="flex cursor-pointer items-center gap-3 rounded-xl border border-dashed p-3 text-left text-xs transition-colors"
                :class="wavDragActive ? 'border-studio-violet bg-studio-violet/5 text-foreground' : 'border-border bg-muted/20 text-muted-foreground hover:border-studio-violet/60'"
                @click="wavInputRef?.click()"
                @keydown.enter.prevent="wavInputRef?.click()"
                @keydown.space.prevent="wavInputRef?.click()"
                @dragover.prevent="wavDragActive = true"
                @dragleave.prevent="wavDragActive = false"
                @drop.prevent="onWavDrop"
              >
                <span class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-studio-violet/10 text-studio-violet"><Upload class="size-4" aria-hidden="true" /></span>
                <span>{{ t('jobUi.wavDropHint') }}</span>
              </div>

              <ul v-if="wavAttachments.length" class="space-y-2">
                <li v-for="att in wavAttachments" :key="att.id" class="rounded-xl border bg-muted/20 p-2.5 text-xs">
                  <div class="flex items-center justify-between gap-2">
                    <div class="flex min-w-0 items-center gap-2">
                      <FileAudio class="size-4 shrink-0 text-studio-violet" aria-hidden="true" />
                      <span class="truncate font-medium">{{ att.name }}</span>
                      <span class="shrink-0 text-muted-foreground">({{ formatBytes(att.sizeBytes) }})</span>
                    </div>
                    <Button variant="ghost" size="icon" type="button" :aria-label="t('jobUi.removeAttachment')" @click="removeWavAttachment(att.id)">
                      <Trash2 class="size-3.5 text-danger" />
                    </Button>
                  </div>
                  <video v-if="att.previewUrl && att.mime_type === 'video/mp4'" controls preload="metadata" :src="att.previewUrl" class="mt-2 max-h-40 w-full rounded" />
                  <audio v-else-if="att.previewUrl" controls preload="metadata" :src="att.previewUrl" class="mt-2 h-8 w-full" />
                </li>
              </ul>
            </div>

            <p v-if="error" role="alert" class="rounded-lg bg-danger/10 px-3 py-2 text-sm text-danger">{{ error }}</p>
          </template>
        </div>
      </form>

      <template #footer>
        <div class="flex flex-wrap items-center justify-between gap-3">
          <p class="flex min-w-0 flex-1 items-center gap-1.5 text-xs text-muted-foreground">
            <ShieldCheck class="size-3.5 shrink-0" aria-hidden="true" />{{ t('jobUi.adminRequester') }}
          </p>
          <div class="flex gap-2">
            <Button variant="outline" type="button" @click="createOpen = false">{{ t('jobUi.close') }}</Button>
            <Button type="submit" :form="createFormId" :disabled="!canSubmit">
              <LoaderCircle v-if="busy" class="size-4 animate-spin" /><Plus v-else class="size-4" />{{ activeRecipe ? t('jobUi.runRecipe') : t('jobUi.create') }}
            </Button>
          </div>
        </div>
      </template>
    </BaseModal>

    <ConfirmDialog
      v-model="batchDeleteOpen"
      :title="t('jobUi.deleteSelectedJobs', { count: selectedIds.length })"
      :confirm-label="t('jobUi.deleteSelectedJobs', { count: selectedIds.length })"
      tone="danger"
      @confirm="deleteSelectedJobs"
    >
      <p>{{ t('jobUi.deleteSelectedConfirm', { count: selectedIds.length }) }}</p>
    </ConfirmDialog>
  </section>
</template>

<style scoped>
@reference "../../style.css";

.jobs-hero {
  background:
    radial-gradient(90% 120% at 100% 0%, rgb(255 255 255 / 0.2), transparent 55%),
    linear-gradient(120deg, #4a2fc4 0%, #6944e9 40%, #0798b9 100%);
}

.jobs-hero-primary {
  @apply inline-flex h-9 items-center gap-2 rounded-full bg-white px-4 text-sm font-semibold text-[#4a2fc4] shadow-sm transition hover:-translate-y-0.5 hover:shadow-md;
}

.jobs-hero-ghost {
  @apply inline-flex h-9 items-center gap-2 rounded-full border border-white/30 bg-white/10 px-3.5 text-sm font-medium backdrop-blur transition hover:bg-white/20 disabled:opacity-60;
}

.jobs-hero-stat {
  @apply rounded-xl border border-white/8 bg-white/10 px-3 py-2.5 backdrop-blur;
}

.jobs-hero-stat dt {
  @apply flex items-center gap-1.5 text-[11px] uppercase tracking-wider text-white/70;
}

.jobs-hero-stat dd {
  @apply mt-1 text-2xl font-semibold tabular-nums;
}

.jobs-chip {
  @apply inline-flex items-center gap-1.5 rounded-full border border-border/80 bg-background px-2.5 py-1 text-xs font-medium text-muted-foreground transition hover:border-studio-violet/50 hover:text-foreground;
}

.jobs-chip-active {
  @apply border-studio-violet bg-studio-violet/10 text-foreground;
}

.jobs-row-live {
  box-shadow: inset 3px 0 0 var(--studio-cyan);
}

.jobs-pick {
  @apply flex w-full items-start gap-3 rounded-xl border border-border p-3 text-left text-sm transition hover:border-studio-violet/60 hover:bg-accent/40;
}

.jobs-pick-active {
  @apply border-studio-violet bg-studio-violet/5 shadow-sm;
}

.jobs-pick-icon {
  @apply flex size-8 shrink-0 items-center justify-center rounded-lg bg-studio-violet/10 text-studio-violet;
}

.jobs-pick-icon-free {
  color: white;
  background: linear-gradient(135deg, #6944e9, #0798b9);
}

.jobs-recipe-head {
  background: linear-gradient(120deg, color-mix(in oklab, var(--studio-violet) 9%, transparent), color-mix(in oklab, var(--studio-cyan) 7%, transparent));
}

.jobs-composer {
  @apply overflow-hidden rounded-xl border border-input bg-background transition;
}

.jobs-composer:focus-within {
  @apply border-ring ring-2 ring-ring/30;
}

.jobs-kbd {
  @apply inline-flex h-5 min-w-5 items-center justify-center rounded border bg-muted px-1 font-sans text-[11px];
}

/* Vertical timeline for the recipe outline. */
.jobs-steps {
  @apply space-y-3;
}

.jobs-step {
  @apply relative flex items-start gap-3;
}

.jobs-step:not(:last-child)::before {
  position: absolute;
  top: 1.75rem;
  bottom: -0.75rem;
  left: 0.875rem;
  width: 1px;
  content: '';
  background: var(--border);
}

.jobs-step-node {
  @apply relative flex size-7 shrink-0 items-center justify-center rounded-full;
}

.jobs-bulk-enter-active,
.jobs-bulk-leave-active {
  transition: opacity 0.18s ease, transform 0.18s ease;
}

.jobs-bulk-enter-from,
.jobs-bulk-leave-to {
  opacity: 0;
  transform: translateY(8px);
}

@media (prefers-reduced-motion: reduce) {
  .jobs-bulk-enter-active,
  .jobs-bulk-leave-active {
    transition: none;
  }
}
</style>

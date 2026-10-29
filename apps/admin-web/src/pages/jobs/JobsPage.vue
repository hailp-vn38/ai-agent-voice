<script setup lang="ts">
import { Activity, ArrowRight, ClipboardList, FileAudio, FolderOpen, Plus, RefreshCw, Search, Settings2, Trash2, Upload, Workflow } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { agentJobsApi } from '@/api/agent-jobs'
import { formatApiError, isApiError } from '@/api/errors'
import type { AdminJob, JobAttachmentUpload, JobStatus } from '@/api/types/agent-jobs'
import PageHeader from '@/components/admin/PageHeader.vue'
import BaseModal from '@/components/admin/BaseModal.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

interface PendingWavAttachment extends JobAttachmentUpload {
  id: string
  sizeBytes: number
  lastModified: number
  previewUrl: string
}

const MAX_WAV_ATTACHMENTS = 4
const MAX_WAV_BYTES = 25 * 1024 * 1024

const router = useRouter()
const { t } = useI18n()
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
const backendPending = ref(false)
const createOpen = ref(false)
const goal = ref('')
const wavAttachments = ref<PendingWavAttachment[]>([])
const wavDragActive = ref(false)
const wavInputRef = ref<HTMLInputElement | null>(null)
const statusOptions: JobStatus[] = ['queued', 'planning', 'running', 'cancelling', 'succeeded', 'partial_success', 'failed', 'cancelled']
let controller: AbortController | undefined
let creation = { sig: '', key: '' }

const activeCount = computed(() => jobs.value.filter(job => ['queued', 'planning', 'running', 'cancelling'].includes(job.status)).length)
const completedCount = computed(() => jobs.value.filter(job => ['succeeded', 'partial_success'].includes(job.status)).length)
const failedCount = computed(() => jobs.value.filter(job => job.status === 'failed').length)
const tone = (status: JobStatus): 'success' | 'danger' | 'secondary' | 'outline' =>
  status === 'succeeded' ? 'success' : status === 'failed' ? 'danger' :
    status === 'partial_success' || status === 'cancelled' ? 'outline' : 'secondary'
const label = (status: JobStatus) => t(('jobUi.jobStatus.' + status) as 'jobUi.jobStatus.queued')
function date(value: string) {
  const stamp = new Date(value)
  return Number.isFinite(stamp.getTime()) ? stamp.toLocaleString() : value
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
    total.value = result.total ?? result.items.length
    page.value = result.page
    hasMore.value = (result.page * result.page_size) < (result.total ?? (result.page * result.page_size + (result.items.length === result.page_size ? 1 : 0)))
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
  }
})

async function create() {
  if (goal.value.trim().length < 5 || busy.value) return
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    const trimmedGoal = goal.value.trim()
    const attSig = wavAttachments.value.map(a => `${a.name}:${a.sizeBytes}:${a.lastModified}`).join('|')
    const nextSig = `${trimmedGoal}::${attSig}`
    if (creation.sig !== nextSig) creation = { sig: nextSig, key: crypto.randomUUID() }
    const uploads: JobAttachmentUpload[] = wavAttachments.value.map(a => ({
      name: a.name,
      mime_type: a.mime_type,
      data_base64: a.data_base64,
    }))
    const job = await agentJobsApi.createJob(trimmedGoal, creation.key, uploads)
    creation = { sig: '', key: '' }
    clearWavAttachments()
    createOpen.value = false
    goal.value = ''
    await router.push({ name: 'job-detail', params: { jobId: job.id } })
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}
function openDetail(id: string) {
  void router.push({ name: 'job-detail', params: { jobId: id } })
}
onMounted(() => { void load() })
onBeforeUnmount(() => {
  controller?.abort()
  clearWavAttachments()
})
</script>

<template>
  <section class="space-y-6">
    <PageHeader :title="t('jobUi.jobs')" :description="t('jobUi.jobsDescription')">
      <template #actions>
        <Button variant="outline" @click="router.push({ name: 'artifacts' })"><FolderOpen class="size-4" />{{ t('nav.artifacts') }}</Button>
        <Button variant="outline" :disabled="loading" @click="load()"><RefreshCw class="size-4" :class="{ 'animate-spin': loading }" />{{ t('jobUi.refresh') }}</Button>
        <Button @click="createOpen = true"><Plus class="size-4" />{{ t('jobUi.newJob') }}</Button>
        <Button variant="outline" size="icon" :aria-label="t('jobUi.background')" :title="t('jobUi.background')" @click="router.push({ name: 'jobs-settings' })"><Settings2 class="size-4" aria-hidden="true" /></Button>
      </template>
    </PageHeader>
    <div v-if="error" role="alert" class="rounded-lg border border-danger/30 bg-danger/5 p-4 text-sm text-danger">
      <p class="font-medium">{{ backendPending ? t('jobUi.backendPending') : t('jobUi.loadFailed') }}</p><p class="mt-1 break-words">{{ error }}</p>
    </div>
    <p v-if="notice" role="status" class="rounded-lg border border-success/30 bg-success/5 p-3 text-sm text-success-foreground">{{ notice }}</p>

    <div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
      <div class="studio-panel flex items-center gap-3 p-4"><ClipboardList class="size-6 text-studio-violet" /><div><p class="text-xl font-semibold tabular-nums">{{ total }}</p><p class="text-xs text-muted-foreground">{{ t('jobUi.totalFiltered') }}</p></div></div>
      <div class="studio-panel flex items-center gap-3 p-4"><Activity class="size-6 text-studio-violet" /><div><p class="text-xl font-semibold tabular-nums">{{ activeCount }}</p><p class="text-xs text-muted-foreground">{{ t('jobUi.activeJobs') }}</p></div></div>
      <div class="studio-panel flex items-center gap-3 p-4"><Workflow class="size-6 text-success" /><div><p class="text-xl font-semibold tabular-nums">{{ completedCount }}</p><p class="text-xs text-muted-foreground">{{ t('jobUi.doneJobs') }}</p></div></div>
      <div class="studio-panel flex items-center gap-3 p-4"><ClipboardList class="size-6 text-danger" /><div><p class="text-xl font-semibold tabular-nums">{{ failedCount }}</p><p class="text-xs text-muted-foreground">{{ t('jobUi.failedJobs') }}</p></div></div>
    </div>

    <div class="flex flex-col gap-2 sm:flex-row">
      <form class="relative min-w-0 flex-1 sm:max-w-sm" @submit.prevent="load()">
        <Search class="pointer-events-none absolute left-3 top-3 size-4 text-muted-foreground" />
        <input v-model="search" class="admin-input pl-9" type="search" :placeholder="t('jobUi.searchJobs')" :aria-label="t('jobUi.searchJobs')" />
      </form>
      <select v-model="filter" class="admin-input sm:w-44" :aria-label="t('jobUi.filterStatus')" @change="load()">
        <option value="all">{{ t('jobUi.allStatus') }}</option>
        <option v-for="status in statusOptions" :key="status" :value="status">{{ label(status) }}</option>
      </select>
      <Button variant="outline" @click="load()">{{ t('jobUi.search') }}</Button>
    </div>

    <div v-if="loading" class="studio-panel p-8 text-center text-sm text-muted-foreground" role="status">{{ t('common.loading') }}</div>
    <div v-else-if="jobs.length" class="studio-panel overflow-hidden">
      <div class="overflow-x-auto">
        <table class="w-full text-left text-sm">
          <thead class="border-b bg-muted/40 text-xs text-muted-foreground">
            <tr>
              <th class="px-4 py-3 font-medium">{{ t('jobUi.goal') }}</th>
              <th class="px-4 py-3 font-medium">{{ t('jobUi.requester') }}</th>
              <th class="px-4 py-3 font-medium">{{ t('jobUi.status') }}</th>
              <th class="px-4 py-3 font-medium">{{ t('jobUi.createdAt') }}</th>
              <th class="px-4 py-3 font-medium"><span class="sr-only">{{ t('jobUi.details') }}</span></th>
            </tr>
          </thead>
          <tbody class="divide-y">
            <tr v-for="job in jobs" :key="job.id" class="hover:bg-muted/25">
              <td class="max-w-sm px-4 py-4">
                <button class="block w-full cursor-pointer truncate text-left font-medium hover:text-studio-violet" type="button" @click="openDetail(job.id)">{{ job.goal }}</button>
                <span v-if="job.execution_mode === 'knowledge_ingest_v3'" class="text-xs text-muted-foreground">{{ t('jobUi.knowledge') }}</span>
                <p class="mt-1 truncate font-mono text-xs text-muted-foreground">{{ job.id }}</p>
              </td>
              <td class="max-w-48 break-all px-4 py-4 text-muted-foreground">{{ job.requester_scope }}</td>
              <td class="px-4 py-4"><Badge :variant="tone(job.status)">{{ label(job.status) }}</Badge></td>
              <td class="whitespace-nowrap px-4 py-4 text-xs text-muted-foreground">{{ date(job.created_at) }}</td>
              <td class="px-4 py-4 text-right"><Button class="cursor-pointer" variant="ghost" size="icon" :aria-label="t('jobUi.details')" @click="openDetail(job.id)"><ArrowRight class="size-4" /></Button></td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
    <div v-else class="studio-panel p-10 text-center text-sm text-muted-foreground">
      <ClipboardList class="mx-auto mb-3 size-8 opacity-60" />{{ t('jobUi.noJobs') }}
    </div>
    <div class="flex justify-end gap-2">
      <Button variant="outline" :disabled="page <= 1 || loading" @click="load(page - 1)">{{ t('jobUi.previous') }}</Button>
      <Button variant="outline" :disabled="!hasMore || loading" @click="load(page + 1)">{{ t('jobUi.next') }}</Button>
    </div>

    <BaseModal v-model="createOpen" :title="t('jobUi.newJob')" :description="t('jobUi.createHint')">
      <form class="space-y-4" @submit.prevent="create">
        <label class="block space-y-2">
          <span class="text-sm font-medium">{{ t('jobUi.goal') }}</span>
          <textarea v-model="goal" class="admin-textarea min-h-32" maxlength="3000" required :placeholder="t('jobUi.goalPlaceholder')" />
        </label>

        <div class="space-y-2">
          <div class="flex items-center justify-between">
            <span class="text-sm font-medium">{{ t('jobUi.wavAttachments') }}</span>
            <span class="text-xs text-muted-foreground">{{ wavAttachments.length }}/{{ MAX_WAV_ATTACHMENTS }}</span>
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
            class="flex cursor-pointer flex-col items-center justify-center gap-2 rounded-lg border border-dashed p-4 text-center text-xs transition-colors"
            :class="wavDragActive ? 'border-studio-violet bg-studio-violet/5 text-foreground' : 'border-border bg-muted/20 text-muted-foreground hover:border-studio-violet/60'"
            @click="wavInputRef?.click()"
            @keydown.enter.prevent="wavInputRef?.click()"
            @keydown.space.prevent="wavInputRef?.click()"
            @dragover.prevent="wavDragActive = true"
            @dragleave.prevent="wavDragActive = false"
            @drop.prevent="onWavDrop"
          >
            <Upload class="size-5 text-studio-violet" />
            <span>{{ t('jobUi.wavDropHint') }}</span>
          </div>

          <ul v-if="wavAttachments.length" class="space-y-2">
            <li
              v-for="att in wavAttachments"
              :key="att.id"
              class="rounded-lg border bg-muted/20 p-2.5 text-xs"
            >
              <div class="flex items-center justify-between gap-2">
                <div class="flex min-w-0 items-center gap-2">
                  <FileAudio class="size-4 shrink-0 text-studio-violet" />
                  <span class="truncate font-medium">{{ att.name }}</span>
                  <span class="shrink-0 text-muted-foreground">({{ formatBytes(att.sizeBytes) }})</span>
                </div>
                <Button
                  variant="ghost"
                  size="icon"
                  type="button"
                  :aria-label="t('jobUi.removeAttachment')"
                  @click="removeWavAttachment(att.id)"
                >
                  <Trash2 class="size-3.5 text-danger" />
                </Button>
              </div>
              <video
                v-if="att.previewUrl && att.mime_type === 'video/mp4'"
                controls
                preload="metadata"
                :src="att.previewUrl"
                class="mt-2 max-h-40 w-full rounded"
              />
              <audio
                v-else-if="att.previewUrl"
                controls
                preload="metadata"
                :src="att.previewUrl"
                class="mt-2 h-8 w-full"
              />
            </li>
          </ul>
        </div>

        <p class="text-xs text-muted-foreground">{{ t('jobUi.adminRequester') }}</p>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <div class="flex justify-end gap-2">
          <Button variant="outline" type="button" @click="createOpen = false">{{ t('jobUi.close') }}</Button>
          <Button type="submit" :disabled="busy || goal.trim().length < 5"><Plus class="size-4" />{{ t('jobUi.create') }}</Button>
        </div>
      </form>
    </BaseModal>
  </section>
</template>

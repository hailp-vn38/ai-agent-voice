<script lang="ts">
import type { AdminArtifactItem, JobArtifact } from '@/api/types/agent-jobs'

export function isMarkdownArtifact(artifact: JobArtifact): boolean {
  const mime = (artifact.media_type || artifact.mime_type || '').toLowerCase()
  const filename = (artifact.filename || artifact.name || '').toLowerCase()
  return (
    mime === 'text/markdown' ||
    mime === 'text/x-markdown' ||
    filename.endsWith('.md') ||
    filename.endsWith('.markdown')
  )
}

export function artifactPreview(artifact: JobArtifact) {
  const mime = (artifact.media_type || artifact.mime_type || '').toLowerCase()
  const filename = artifact.filename || artifact.name || ''
  if (['image/png', 'image/jpeg', 'image/webp', 'image/gif'].includes(mime)) {
    return { kind: 'image', limit: 64 * 1024 * 1024 } as const
  }
  if (['audio/mpeg', 'audio/wav', 'audio/ogg', 'audio/flac', 'audio/x-wav', 'audio/aac'].includes(mime)) {
    return { kind: 'audio', limit: 64 * 1024 * 1024 } as const
  }
  if (['video/mp4', 'video/webm', 'video/quicktime'].includes(mime)) {
    return { kind: 'video', limit: 64 * 1024 * 1024 } as const
  }
  if (
    mime.startsWith('text/') ||
    ['application/json', 'application/sql', 'application/javascript', 'application/xml'].includes(mime) ||
    /\.(rs|py|js|ts|tsx|jsx|html|css|sql|json|jsonl|csv|tsv|xml|md|markdown|txt)$/i.test(filename)
  ) {
    return { kind: 'text', limit: 1024 * 1024 } as const
  }
  return { kind: 'download', limit: 0 } as const
}
</script>

<script setup lang="ts">
import {
  ChevronLeft,
  ChevronRight,
  Download,
  ExternalLink,
  FileCode2,
  FileText,
  Film,
  Image as ImageIcon,
  Music,
  ShieldCheck,
  X,
} from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { agentJobsApi } from '@/api/agent-jobs'
import { formatApiError } from '@/api/errors'
import type { ArtifactKind, ArtifactRole, JobStatus } from '@/api/types/agent-jobs'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { renderMarkdown } from '@/lib/markdown'

const { t } = useI18n()
const props = withDefaults(
  defineProps<{
    jobId: string
    artifact: JobArtifact | AdminArtifactItem
    hasPrev?: boolean
    hasNext?: boolean
    showNav?: boolean
  }>(),
  {
    hasPrev: false,
    hasNext: false,
    showNav: false,
  },
)
const emit = defineEmits<{
  close: []
  download: []
  prev: []
  next: []
  openJob: [jobId: string]
}>()

const loading = ref(true)
const error = ref('')
const url = ref('')
const text = ref('')
const textViewMode = ref<'rendered' | 'raw'>('rendered')
let active = true

const preview = computed(() => artifactPreview(props.artifact))
const kind = computed(() => preview.value.kind)
const isMarkdown = computed(() => isMarkdownArtifact(props.artifact))
const renderedHtml = computed(() => (isMarkdown.value && text.value ? renderMarkdown(text.value) : ''))

const catalogArtifact = computed<Partial<AdminArtifactItem>>(() => props.artifact as Partial<AdminArtifactItem>)

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'
  if (bytes < 1024) return `${bytes} B`
  const kib = bytes / 1024
  if (kib < 1024) return `${kib.toFixed(1)} KiB`
  const mib = kib / 1024
  if (mib < 1024) return `${mib.toFixed(2)} MiB`
  return `${(mib / 1024).toFixed(2)} GiB`
}

function formatDate(value?: string): string {
  if (!value) return '—'
  const num = Number(value)
  const stamp =
    Number.isFinite(num) && num > 0
      ? new Date(num < 1_000_000_000_000 ? num * 1000 : num)
      : new Date(value)
  return Number.isFinite(stamp.getTime()) ? stamp.toLocaleString() : value
}

function kindLabel(artifactKind?: string): string {
  if (!artifactKind) return '—'
  return t(('artifactsUi.kind.' + artifactKind) as 'artifactsUi.kind.report')
}

function roleLabel(role?: string): string {
  if (!role) return '—'
  return role === 'primary' ? t('artifactsUi.role.primary') : t('artifactsUi.role.intermediate')
}

function jobTone(status?: JobStatus): 'success' | 'danger' | 'secondary' | 'outline' {
  return status === 'succeeded'
    ? 'success'
    : status === 'failed'
      ? 'danger'
      : status === 'partial_success' || status === 'cancelled'
        ? 'outline'
        : 'secondary'
}

function jobStatusLabel(status?: JobStatus): string {
  if (!status) return ''
  return t(('jobUi.jobStatus.' + status) as 'jobUi.jobStatus.queued')
}

function headerIcon() {
  switch (props.artifact.kind) {
    case 'report':
      return FileText
    case 'image':
      return ImageIcon
    case 'video':
      return Film
    case 'audio':
      return Music
    default:
      return FileCode2
  }
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    emit('close')
  } else if (event.key === 'ArrowLeft' && props.showNav && props.hasPrev) {
    emit('prev')
  } else if (event.key === 'ArrowRight' && props.showNav && props.hasNext) {
    emit('next')
  }
}

onMounted(async () => {
  window.addEventListener('keydown', onKeydown)
  if (kind.value === 'download') {
    loading.value = false
    return
  }
  try {
    const file = await agentJobsApi.artifact(props.jobId, props.artifact.id)
    if (!active) return
    const cap = preview.value.limit
    if (file.size > cap) {
      error.value = 'File exceeds preview limit. Use Download.'
      return
    }
    if (kind.value === 'text') {
      text.value = await file.text()
    } else {
      const effectiveMime = props.artifact.media_type || props.artifact.mime_type || 'application/octet-stream'
      url.value = URL.createObjectURL(new Blob([file], { type: effectiveMime }))
    }
  } catch (cause) {
    if (active) error.value = formatApiError(cause)
  } finally {
    if (active) loading.value = false
  }
})

onBeforeUnmount(() => {
  active = false
  window.removeEventListener('keydown', onKeydown)
  if (url.value) URL.revokeObjectURL(url.value)
})
</script>

<template>
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4 backdrop-blur-[2px] sm:p-6"
    role="presentation"
    data-artifact-modal-backdrop
    @click.self="emit('close')"
  >
    <section
      role="dialog"
      aria-modal="true"
      :aria-label="artifact.filename || artifact.name || artifact.id"
      data-artifact-detail-modal
      class="studio-panel flex max-h-[90vh] w-full max-w-5xl flex-col overflow-hidden rounded-xl border bg-background shadow-2xl"
    >
      <header class="flex shrink-0 items-center justify-between gap-4 border-b bg-muted/20 px-5 py-3.5">
        <div class="flex min-w-0 items-center gap-3">
          <div class="grid size-9 shrink-0 place-items-center rounded-lg bg-studio-violet/15 text-studio-violet">
            <component :is="headerIcon()" class="size-4.5" />
          </div>
          <div class="min-w-0">
            <h3 class="truncate text-sm font-semibold sm:text-base">
              {{ artifact.filename || artifact.name || artifact.id }}
            </h3>
            <p class="truncate text-xs text-muted-foreground">
              <span class="font-mono">{{ artifact.media_type || artifact.mime_type }}</span>
              <span> · {{ formatBytes(artifact.size_bytes) }}</span>
            </p>
          </div>
        </div>

        <div class="flex shrink-0 items-center gap-2">
          <div
            v-if="kind === 'text' && isMarkdown && !loading && !error"
            class="inline-flex rounded-lg border bg-muted/40 p-0.5 text-xs"
          >
            <button
              type="button"
              class="rounded-md px-2.5 py-1 font-medium transition-colors"
              :class="
                textViewMode === 'rendered'
                  ? 'bg-background text-foreground shadow-sm'
                  : 'text-muted-foreground hover:text-foreground'
              "
              data-preview-mode-rendered
              @click="textViewMode = 'rendered'"
            >
              {{ t('jobUi.promptModePreview') }}
            </button>
            <button
              type="button"
              class="rounded-md px-2.5 py-1 font-medium transition-colors"
              :class="
                textViewMode === 'raw'
                  ? 'bg-background text-foreground shadow-sm'
                  : 'text-muted-foreground hover:text-foreground'
              "
              data-preview-mode-raw
              @click="textViewMode = 'raw'"
            >
              Raw
            </button>
          </div>

          <div v-if="showNav" class="inline-flex items-center gap-1">
            <Button
              variant="outline"
              size="icon"
              class="size-8"
              data-modal-prev
              :disabled="!hasPrev"
              :aria-label="t('artifactsUi.previousItem')"
              :title="t('artifactsUi.previousItem')"
              @click="emit('prev')"
            >
              <ChevronLeft class="size-4" />
            </Button>
            <Button
              variant="outline"
              size="icon"
              class="size-8"
              data-modal-next
              :disabled="!hasNext"
              :aria-label="t('artifactsUi.nextItem')"
              :title="t('artifactsUi.nextItem')"
              @click="emit('next')"
            >
              <ChevronRight class="size-4" />
            </Button>
          </div>

          <Button variant="outline" size="icon" class="size-8" aria-label="Close preview" @click="emit('close')">
            <X class="size-4" />
          </Button>
        </div>
      </header>

      <div class="grid min-h-0 flex-1 gap-0 overflow-y-auto lg:grid-cols-[minmax(0,1fr)_19rem] lg:divide-x">
        <div class="flex min-h-[20rem] flex-col justify-center overflow-y-auto p-5">
          <div
            v-if="loading"
            role="status"
            aria-busy="true"
            class="card-shimmer space-y-3 rounded-lg border border-border/60 bg-muted/20 p-5"
          >
            <span class="sr-only">Loading preview…</span>
            <div class="h-4 w-48 rounded bg-muted/60" aria-hidden="true" />
            <div class="h-44 w-full rounded-lg bg-muted/40" aria-hidden="true" />
          </div>

          <p v-else-if="error" role="alert" class="rounded-lg border border-danger/30 bg-danger/5 p-4 text-sm text-danger">
            {{ error }}
          </p>

          <div
            v-else-if="kind === 'download'"
            data-preview-download-only
            class="mx-auto flex max-w-md flex-col items-center justify-center gap-3 rounded-xl border border-dashed bg-muted/20 p-8 text-center"
          >
            <component :is="headerIcon()" class="size-10 text-muted-foreground" />
            <p class="text-sm font-medium">{{ artifact.filename || artifact.name || artifact.id }}</p>
            <p class="text-xs text-muted-foreground">{{ t('artifactsUi.downloadOnlyHint') }}</p>
            <Button size="sm" class="mt-1" @click="emit('download')">
              <Download class="size-4" />{{ t('jobUi.download') }}
            </Button>
          </div>

          <div
            v-else-if="kind === 'audio'"
            class="mx-auto flex w-full max-w-lg flex-col items-center gap-4 rounded-xl border bg-muted/20 p-6"
          >
            <div class="grid size-14 place-items-center rounded-full bg-studio-cyan/15 text-studio-cyan">
              <Music class="size-7" />
            </div>
            <p class="truncate text-sm font-medium">{{ artifact.filename || artifact.name }}</p>
            <audio :src="url" controls class="w-full" />
          </div>

          <video
            v-else-if="kind === 'video'"
            :src="url"
            controls
            class="max-h-[65vh] w-full rounded-lg bg-black/40 object-contain"
          />

          <img
            v-else-if="kind === 'image'"
            :src="url"
            :alt="artifact.filename || artifact.name || 'Artifact image'"
            class="max-h-[65vh] w-full rounded-lg object-contain"
          />

          <article
            v-else-if="isMarkdown && textViewMode === 'rendered'"
            data-preview-markdown-body
            class="job-report max-h-[65vh] overflow-auto rounded-lg border bg-muted/25 p-5 text-sm leading-6"
            v-html="renderedHtml"
          />

          <pre
            v-else
            class="max-h-[65vh] overflow-auto whitespace-pre-wrap break-all rounded-lg border bg-muted/40 p-4 font-mono text-xs"
          >{{ text }}</pre>
        </div>

        <aside class="space-y-4 bg-muted/10 p-5 text-xs" data-artifact-detail-meta>
          <div>
            <h4 class="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
              {{ t('artifactsUi.metadata') }}
            </h4>
            <div class="mt-2.5 flex flex-wrap items-center gap-1.5">
              <Badge v-if="artifact.kind" variant="secondary">{{ kindLabel(artifact.kind) }}</Badge>
              <Badge v-if="artifact.role" :variant="artifact.role === 'primary' ? 'success' : 'outline'">
                {{ roleLabel(artifact.role) }}
              </Badge>
            </div>
          </div>

          <dl class="space-y-2.5 border-t pt-3.5">
            <div class="flex items-start justify-between gap-2">
              <dt class="text-muted-foreground">{{ t('artifactsUi.file') }}</dt>
              <dd class="max-w-[11rem] truncate font-mono font-medium" :title="artifact.filename || artifact.name">
                {{ artifact.filename || artifact.name || artifact.id }}
              </dd>
            </div>
            <div class="flex items-start justify-between gap-2">
              <dt class="text-muted-foreground">{{ t('artifactsUi.size') }}</dt>
              <dd class="font-mono tabular-nums">{{ formatBytes(artifact.size_bytes) }}</dd>
            </div>
            <div class="flex items-start justify-between gap-2">
              <dt class="text-muted-foreground">{{ t('jobUi.createdAt') }}</dt>
              <dd class="text-right">{{ formatDate(artifact.created_at) }}</dd>
            </div>
            <div class="flex items-start justify-between gap-2">
              <dt class="text-muted-foreground">{{ t('artifactsUi.producedBy') }}</dt>
              <dd class="max-w-[11rem] truncate font-mono font-medium">
                {{ artifact.producer_agent_id || catalogArtifact.root_agent_id || '—' }}
              </dd>
            </div>
            <div v-if="artifact.producer_tool_name" class="flex items-start justify-between gap-2">
              <dt class="text-muted-foreground">{{ t('artifactsUi.toolUsed') }}</dt>
              <dd class="truncate font-mono">{{ artifact.producer_tool_name }}</dd>
            </div>
          </dl>

          <div v-if="jobId" class="space-y-2 border-t pt-3.5">
            <div class="flex items-center justify-between gap-2">
              <span class="text-muted-foreground">{{ t('artifactsUi.job') }}</span>
              <Badge v-if="catalogArtifact.job_status" :variant="jobTone(catalogArtifact.job_status)" class="text-[10px]">
                {{ jobStatusLabel(catalogArtifact.job_status) }}
              </Badge>
            </div>
            <button
              type="button"
              data-artifact-job-link
              class="flex w-full cursor-pointer items-center justify-between gap-2 rounded-lg border bg-background/80 px-3 py-2 text-left font-medium text-studio-violet transition-colors hover:border-studio-violet/50 hover:bg-studio-violet/5"
              @click="emit('openJob', jobId)"
            >
              <span class="truncate">{{ catalogArtifact.job_title || catalogArtifact.job_goal || jobId }}</span>
              <ExternalLink class="size-3.5 shrink-0" />
            </button>
          </div>

          <div v-if="artifact.sha256" class="space-y-1 border-t pt-3.5">
            <div class="flex items-center gap-1 text-muted-foreground">
              <ShieldCheck class="size-3.5 text-success" />
              <span>{{ t('artifactsUi.checksum') }}</span>
            </div>
            <p class="break-all rounded bg-muted/40 p-2 font-mono text-[10px] text-muted-foreground">
              {{ artifact.sha256 }}
            </p>
          </div>
        </aside>
      </div>

      <footer class="flex shrink-0 items-center justify-between gap-2 border-t bg-muted/20 px-5 py-3">
        <div>
          <Button
            v-if="jobId"
            variant="outline"
            size="sm"
            data-artifact-open-job-btn
            @click="emit('openJob', jobId)"
          >
            <ExternalLink class="size-3.5" />{{ t('artifactsUi.openJob') }}
          </Button>
        </div>
        <div class="flex items-center gap-2">
          <Button variant="outline" size="sm" data-artifact-modal-download-btn @click="emit('download')">
            <Download class="size-4" />{{ t('jobUi.download') }}
          </Button>
          <Button size="sm" @click="emit('close')">{{ t('common.close') }}</Button>
        </div>
      </footer>
    </section>
  </div>
</template>

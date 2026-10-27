<script lang="ts">
import type { JobArtifact } from '@/api/types/agent-jobs'

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
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { Download, X } from '@lucide/vue'
import { agentJobsApi } from '@/api/agent-jobs'
import { formatApiError } from '@/api/errors'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { renderMarkdown } from '@/lib/markdown'

const { t } = useI18n()
const props = defineProps<{ jobId: string; artifact: JobArtifact }>()
const emit = defineEmits<{ close: []; download: [] }>()
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

onMounted(async () => {
  try {
    const file = await agentJobsApi.artifact(props.jobId, props.artifact.id)
    if (!active) return
    const cap = preview.value.limit
    if (file.size > cap) { error.value = 'File exceeds preview limit. Use Download.'; return }
    if (kind.value === 'text') text.value = await file.text()
    else if (kind.value !== 'download') {
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
  if (url.value) URL.revokeObjectURL(url.value)
})
</script>

<template>
  <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/65 p-4" role="presentation" @click.self="emit('close')">
    <section role="dialog" aria-modal="true" :aria-label="artifact.filename || artifact.name || artifact.id"
      class="w-full max-w-3xl space-y-4 rounded-xl border bg-background p-5 shadow-xl">
      <header class="flex items-center justify-between gap-4">
        <div class="min-w-0">
          <h3 class="truncate font-semibold">{{ artifact.filename || artifact.name || artifact.id }}</h3>
          <p class="text-xs text-muted-foreground">
            {{ artifact.media_type || artifact.mime_type }}
            <span v-if="artifact.producer_agent_id"> · {{ artifact.producer_agent_id }}</span>
            <span v-if="artifact.producer_tool_name"> ({{ artifact.producer_tool_name }})</span>
          </p>
        </div>
        <div class="flex items-center gap-2">
          <div v-if="kind === 'text' && isMarkdown && !loading && !error" class="inline-flex rounded-lg border bg-muted/40 p-0.5 text-xs">
            <button
              type="button"
              class="rounded-md px-2.5 py-1 font-medium transition-colors"
              :class="textViewMode === 'rendered' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'"
              data-preview-mode-rendered
              @click="textViewMode = 'rendered'"
            >
              {{ t('jobUi.promptModePreview') }}
            </button>
            <button
              type="button"
              class="rounded-md px-2.5 py-1 font-medium transition-colors"
              :class="textViewMode === 'raw' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'"
              data-preview-mode-raw
              @click="textViewMode = 'raw'"
            >
              Raw
            </button>
          </div>
          <Button variant="outline" size="sm" aria-label="Close preview" @click="emit('close')"><X class="size-4" /></Button>
        </div>
      </header>
      <div v-if="loading" role="status" aria-busy="true" class="card-shimmer space-y-3 rounded-lg border border-border/60 bg-muted/20 p-5">
        <span class="sr-only">Loading preview…</span>
        <div class="h-4 w-48 rounded bg-muted/60" aria-hidden="true" />
        <div class="h-36 w-full rounded-lg bg-muted/40" aria-hidden="true" />
      </div>
      <p v-else-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
      <p v-else-if="kind === 'download'" class="text-sm text-muted-foreground">Preview unavailable for this file.</p>
      <audio v-else-if="kind === 'audio'" :src="url" controls class="w-full" />
      <video v-else-if="kind === 'video'" :src="url" controls class="max-h-[65vh] w-full rounded-lg" />
      <img v-else-if="kind === 'image'" :src="url" :alt="artifact.filename || artifact.name || 'Artifact image'" class="max-h-[65vh] w-full object-contain" />
      <article
        v-else-if="isMarkdown && textViewMode === 'rendered'"
        data-preview-markdown-body
        class="job-report max-h-[65vh] overflow-auto rounded-lg bg-muted/40 p-4 text-sm leading-6"
        v-html="renderedHtml"
      />
      <pre v-else class="max-h-[65vh] overflow-auto whitespace-pre-wrap break-all rounded-lg bg-muted/50 p-4 text-xs">{{ text }}</pre>
      <footer class="flex justify-end gap-2">
        <Button variant="outline" size="sm" @click="emit('download')"><Download class="size-4" />{{ t('jobUi.download') }}</Button>
        <Button size="sm" @click="emit('close')">{{ t('common.close') }}</Button>
      </footer>
    </section>
  </div>
</template>

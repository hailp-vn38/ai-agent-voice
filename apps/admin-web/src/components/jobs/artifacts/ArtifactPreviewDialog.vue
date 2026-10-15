<script lang="ts">
import type { JobArtifact } from '@/api/types/agent-jobs'

export function artifactPreview(artifact: JobArtifact) {
  const mime = artifact.media_type.toLowerCase()
  if (['image/png', 'image/jpeg', 'image/webp', 'image/gif'].includes(mime)) return { kind: 'image', limit: 64 * 1024 * 1024 } as const
  if (['audio/mpeg', 'audio/wav', 'audio/ogg', 'audio/flac', 'audio/x-wav'].includes(mime)) return { kind: 'audio', limit: 64 * 1024 * 1024 } as const
  if (['video/mp4', 'video/webm'].includes(mime)) return { kind: 'video', limit: 64 * 1024 * 1024 } as const
  if (mime.startsWith('text/') || ['application/json', 'application/sql', 'application/javascript'].includes(mime)
    || /\.(rs|py|js|ts|tsx|jsx|html|css|sql|json|csv|md|txt)$/i.test(artifact.filename)) return { kind: 'text', limit: 1024 * 1024 } as const
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

const { t } = useI18n()
const props = defineProps<{ jobId: string; artifact: JobArtifact }>()
const emit = defineEmits<{ close: []; download: [] }>()
const loading = ref(true)
const error = ref('')
const url = ref('')
const text = ref('')
let active = true
const preview = computed(() => artifactPreview(props.artifact))
const kind = computed(() => preview.value.kind)
onMounted(async () => {
  try {
    const file = await agentJobsApi.artifact(props.jobId, props.artifact.id)
    if (!active) return
    const cap = preview.value.limit
    if (file.size > cap) { error.value = 'File exceeds preview limit. Use Download.'; return }
    if (kind.value === 'text') text.value = await file.text()
    else if (kind.value !== 'download') url.value = URL.createObjectURL(new Blob([file], { type: props.artifact.media_type }))
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
    <section role="dialog" aria-modal="true" :aria-label="artifact.filename || artifact.id"
      class="w-full max-w-3xl space-y-4 rounded-xl border bg-background p-5 shadow-xl">
      <header class="flex items-center justify-between gap-4">
        <div class="min-w-0">
          <h3 class="truncate font-semibold">{{ artifact.filename || artifact.id }}</h3>
          <p class="text-xs text-muted-foreground">{{ artifact.media_type }}</p>
        </div>
        <Button variant="outline" size="sm" aria-label="Close preview" @click="emit('close')"><X class="size-4" /></Button>
      </header>
      <p v-if="loading" class="text-sm text-muted-foreground">Loading preview…</p>
      <p v-else-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
      <p v-else-if="kind === 'download'" class="text-sm text-muted-foreground">Preview unavailable for this file.</p>
      <audio v-else-if="kind === 'audio'" :src="url" controls class="w-full" />
      <video v-else-if="kind === 'video'" :src="url" controls class="max-h-[65vh] w-full rounded-lg" />
      <img v-else-if="kind === 'image'" :src="url" :alt="artifact.filename || 'Artifact image'" class="max-h-[65vh] w-full object-contain" />
      <pre v-else class="max-h-[65vh] overflow-auto whitespace-pre-wrap break-all rounded-lg bg-muted/50 p-4 text-xs">{{ text }}</pre>
      <footer class="flex justify-end gap-2">
        <Button variant="outline" size="sm" @click="emit('download')"><Download class="size-4" />{{ t('jobUi.download') }}</Button>
        <Button size="sm" @click="emit('close')">Close</Button>
      </footer>
    </section>
  </div>
</template>

<script setup lang="ts">
import { ref, watch } from 'vue'
import { Download, Eye, File } from '@lucide/vue'
import { agentJobsApi } from '@/api/agent-jobs'
import { formatApiError } from '@/api/errors'
import type { AdminJob, JobArtifact } from '@/api/types/agent-jobs'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import ArtifactPreviewDialog from './ArtifactPreviewDialog.vue'

const { t } = useI18n()
const props = defineProps<{ job: AdminJob }>()
const selected = ref<JobArtifact | null>(null)
const error = ref('')
const downloading = ref<string | null>(null)
watch(() => props.job.id, () => { selected.value = null; error.value = '' })

function canPreview(item: JobArtifact) {
  const mime = item.media_type.toLowerCase()
  const text = mime.startsWith('text/') || ['application/json', 'application/sql', 'application/javascript'].includes(mime)
    || /\.(rs|py|js|ts|tsx|jsx|html|css|sql|json|csv|md|txt)$/i.test(item.filename)
  const media = [
    'image/png', 'image/jpeg', 'image/webp', 'image/gif',
    'audio/mpeg', 'audio/wav', 'audio/x-wav', 'audio/ogg', 'audio/flac',
    'video/mp4', 'video/webm',
  ].includes(mime)
  return item.size_bytes >= 0 && item.size_bytes <= (text ? 1024 * 1024 : 64 * 1024 * 1024) && (text || media)
}
function origin(item: JobArtifact) {
  const step = props.job.steps.find(row => row.step_key === item.step_key)
  const run = props.job.agent_runs?.find(row => row.agent_run_id === step?.agent_run_id)
  return [run?.role, item.step_key].filter(Boolean).join(' · ') || 'Job'
}
async function download(item: JobArtifact) {
  if (downloading.value) return
  downloading.value = item.id
  error.value = ''
  try {
    const data = await agentJobsApi.artifact(props.job.id, item.id)
    const url = URL.createObjectURL(data)
    const link = document.createElement('a')
    link.href = url
    link.download = item.filename || item.id
    link.click()
    window.setTimeout(() => URL.revokeObjectURL(url), 1000)
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    downloading.value = null
  }
}
</script>

<template>
  <div v-if="!job.artifacts.length" class="text-sm text-muted-foreground">{{ t('jobUi.noArtifacts') }}</div>
  <div v-else class="space-y-1">
    <div v-for="item in job.artifacts" :key="item.id" class="flex items-center justify-between gap-3 border-t py-3">
      <div class="flex min-w-0 items-center gap-3">
        <div class="grid size-9 shrink-0 place-items-center rounded-lg bg-muted/60"><File class="size-4" /></div>
        <div class="min-w-0">
          <p class="truncate text-sm font-medium" :title="item.filename || item.id">{{ item.filename || item.id }}</p>
          <p class="mt-1 truncate text-xs text-muted-foreground">{{ item.media_type }} · {{ (item.size_bytes / 1024).toFixed(1) }} KiB</p>
          <p class="truncate text-xs text-muted-foreground">{{ origin(item) }}</p>
        </div>
      </div>
      <div class="flex shrink-0 gap-2">
        <Button variant="outline" size="sm" :disabled="downloading !== null" @click="download(item)">
          <Download class="size-4" />{{ t('jobUi.download') }}
        </Button>
        <Button v-if="canPreview(item)" variant="outline" size="sm" @click="selected = item"><Eye class="size-4" />Preview</Button>
      </div>
    </div>
  </div>
  <p v-if="error" role="alert" class="mt-2 text-sm text-danger">{{ error }}</p>
  <ArtifactPreviewDialog v-if="selected" :key="selected.id" :job-id="job.id" :artifact="selected"
    @close="selected = null" @download="selected && download(selected)" />
</template>

<script setup lang="ts">
import { CheckCircle } from '@lucide/vue'
import { onBeforeUnmount, ref, watch } from 'vue'

import { formatApiError } from '@/api/errors'
import { speakersApi } from '@/api/speakers'
import type { Speaker, SpeakerCapture } from '@/api/types/speakers'
import BaseModal from '@/components/admin/BaseModal.vue'
import { Button } from '@/components/ui/button'
import { useMicrophoneRecorder } from '@/composables/useMicrophoneRecorder'
import VoiceRecordingDock from '@/components/voice/VoiceRecordingDock.vue'

const props = defineProps<{ open: boolean; speakerKey?: string }>()
const emit = defineEmits<{ 'update:open': [boolean]; completed: [Speaker] }>()
const recorder = useMicrophoneRecorder()
const limits = ref({ minClipMs: 5_000, maxClipMs: 10_000 })
const step = ref<'record' | 'result' | 'details'>('record')
const capture = ref<SpeakerCapture>()
const speaker = ref<Speaker>()
const available = ref(false)
const loading = ref(false)
const uploading = ref(false)
const saving = ref(false)
const completed = ref(false)
const error = ref('')
const name = ref('')
const description = ref('')
let controller: AbortController | undefined

async function load() {
  loading.value = true
  error.value = ''
  try {
    const [summary, existing] = await Promise.all([
      speakersApi.summary(),
      props.speakerKey ? speakersApi.get(props.speakerKey) : Promise.resolve(undefined),
    ])
    available.value = summary.available
    limits.value = {
      minClipMs: summary.enrollment.min_clip_ms,
      maxClipMs: summary.enrollment.max_clip_ms,
    }
    speaker.value = existing
    name.value = existing?.name ?? ''
    description.value = existing?.description ?? ''
    if (!available.value) error.value = 'Bộ nhận dạng giọng nói hiện không khả dụng.'
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    loading.value = false
  }
}

async function startRecording() {
  if (!available.value || loading.value || uploading.value || recorder.starting.value || recorder.recording.value) return
  error.value = ''
  await recorder.start(limits.value, () => void stopRecording())
  if (recorder.error.value) {
    error.value = 'Không thể sử dụng microphone. Hãy cấp quyền rồi thử lại.'
  }
}

async function stopRecording() {
  if (uploading.value) return
  const wav = await recorder.stop()
  if (!wav) {
    error.value = 'Không có âm thanh, vui lòng ghi âm lại.'
    return
  }
  uploading.value = true
  error.value = ''
  controller?.abort()
  controller = new AbortController()
  try {
    capture.value = await speakersApi.capture(wav, controller.signal)
    step.value = 'result'
  } catch (cause) {
    if (!controller.signal.aborted) error.value = formatApiError(cause)
  } finally {
    uploading.value = false
  }
}

function retry() {
  capture.value = undefined
  step.value = 'record'
  error.value = ''
}

async function save() {
  if (!capture.value || saving.value) return
  if (!props.speakerKey && !name.value.trim()) return
  saving.value = true
  error.value = ''
  try {
    const result = props.speakerKey
      ? await speakersApi.replaceVoiceprint(props.speakerKey, capture.value.capture_id, speaker.value!.revision)
      : await speakersApi.createFromCapture({
          capture_id: capture.value.capture_id,
          name: name.value.trim(),
          description: description.value.trim() || undefined,
        })
    completed.value = true
    emit('completed', result.speaker)
  } catch (cause) {
    error.value = formatApiError(cause)
    if (/capture_(not_found|expired)|capture_runtime_incompatible|speaker_embedding_space_changed/.test(error.value)) {
      retry()
    }
  } finally {
    saving.value = false
  }
}

function close() { emit('update:open', false) }
function reset() {
  controller?.abort()
  controller = undefined
  recorder.dispose()
  step.value = 'record'
  capture.value = undefined
  speaker.value = undefined
  completed.value = false
  error.value = ''
  name.value = ''
  description.value = ''
  available.value = false
}
watch(() => props.open, (open) => { if (open) void load(); else reset() })
onBeforeUnmount(reset)
</script>

<template>
  <BaseModal
    :model-value="open"
    :title="speakerKey ? 'Thu lại giọng nói' : 'Thêm người nói'"
    description="Thu một mẫu giọng nói để nhận dạng trong hội thoại, không dùng để xác thực."
    @update:model-value="close"
  >
    <div class="space-y-5">
      <div class="flex justify-between text-sm text-muted-foreground">
        <span :class="step === 'record' ? 'font-medium text-foreground' : ''">1 Ghi âm</span>
        <span :class="step === 'result' ? 'font-medium text-foreground' : ''">2 Trích xuất</span>
        <span :class="step === 'details' ? 'font-medium text-foreground' : ''">3 Lưu người nói</span>
      </div>
      <p v-if="error" role="alert" class="rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">{{ error }}</p>
      <div v-if="loading" class="py-6 text-center text-sm text-muted-foreground">Đang tải cấu hình thu âm…</div>
      <template v-else-if="!completed && step === 'record'">
        <VoiceRecordingDock
          :recording="recorder.recording.value"
          :starting="recorder.starting.value"
          :paused="recorder.paused.value"
          :elapsed-ms="recorder.elapsedMs.value"
          :level="recorder.level.value"
          :min-clip-ms="limits.minClipMs"
          :max-clip-ms="limits.maxClipMs"
          :busy="uploading"
          :disabled="!available"
          @start="startRecording"
          @stop="stopRecording"
        />
      </template>
      <div v-else-if="!completed && step === 'result' && capture" class="py-5 text-center">
        <CheckCircle class="mx-auto size-10 text-emerald-600" />
        <p class="mt-3 font-medium">Trích xuất mẫu giọng nói thành công</p>
        <p class="mt-1 text-sm text-muted-foreground">
          Thời lượng: {{ (capture.quality.duration_ms / 1000).toFixed(1) }}s ·
          Tiếng nói: {{ (capture.quality.speech_ms / 1000).toFixed(1) }}s
        </p>
      </div>
      <div v-else-if="!completed && step === 'details'" class="space-y-3">
        <template v-if="!speakerKey">
          <label class="block space-y-1">
            <span class="text-sm font-medium">Tên người nói *</span>
            <input v-model="name" class="admin-input" maxlength="128" required />
          </label>
          <label class="block space-y-1">
            <span class="text-sm font-medium">Mô tả (không bắt buộc)</span>
            <textarea v-model="description" class="admin-input" rows="3" maxlength="2048" />
          </label>
        </template>
        <p v-else class="text-sm text-muted-foreground">
          Mẫu giọng nói mới sẽ thay thế mẫu hiện tại của {{ speaker?.name }}. Thông tin và liên kết Agent được giữ nguyên.
        </p>
      </div>
      <div v-else class="py-5 text-center">
        <CheckCircle class="mx-auto size-10 text-emerald-600" />
        <p class="mt-3 font-medium">Đã lưu mẫu giọng nói thành công</p>
      </div>
    </div>
    <template v-if="completed || step !== 'record'" #footer>
      <div class="flex flex-wrap justify-end gap-2">
        <Button v-if="completed" variant="outline" @click="close">Đóng</Button>
        <template v-else-if="step === 'result'">
          <Button variant="outline" @click="retry">Ghi âm lại</Button>
          <Button @click="step = 'details'">Tiếp tục</Button>
        </template>
        <template v-else>
          <Button variant="outline" :disabled="saving" @click="step = 'result'">Quay lại</Button>
          <Button :disabled="saving || (!speakerKey && !name.trim())" @click="save">
            {{ saving ? 'Đang lưu…' : 'Lưu mẫu' }}
          </Button>
        </template>
      </div>
    </template>
  </BaseModal>
</template>

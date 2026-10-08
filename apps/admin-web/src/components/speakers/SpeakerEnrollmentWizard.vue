<script setup lang="ts">
import { CheckCircle, Mic, Square, Trash2, X } from '@lucide/vue'
import { computed, onBeforeUnmount, ref, watch } from 'vue'

import { formatApiError } from '@/api/errors'
import { providersApi } from '@/api/providers'
import { speakersApi } from '@/api/speakers'
import type { AdminProvider } from '@/api/types/providers'
import type { EnrollmentDraft, Speaker } from '@/api/types/speakers'
import BaseModal from '@/components/admin/BaseModal.vue'
import QuickSpeakerEnrollment from '@/components/speakers/QuickSpeakerEnrollment.vue'
import { Button } from '@/components/ui/button'
import { useMicrophoneRecorder } from '@/composables/useMicrophoneRecorder'

const props = withDefaults(defineProps<{
  open: boolean
  speakerKey?: string
  defaultMode?: 'validated' | 'quick'
}>(), { defaultMode: 'validated' })
const emit = defineEmits<{ 'update:open': [boolean]; completed: [Speaker] }>()

const mode = ref<'validated' | 'quick'>('validated')
const providers = ref<AdminProvider[]>([])
const providerKey = ref('')
const name = ref('')
const description = ref('')
const speaker = ref<Speaker>()
const draft = ref<EnrollmentDraft>()
const limits = ref({ minClipMs: 5_000, maxClipMs: 10_000, minSamples: 3, maxSamples: 5 })
const loading = ref(false)
const saving = ref(false)
const uploading = ref(false)
const validating = ref(false)
const error = ref('')
const completed = ref(false)
const recorder = useMicrophoneRecorder()
const holdoutRecorder = useMicrophoneRecorder()
let uploadAbort: AbortController | undefined
let operationAbort: AbortController | undefined
let stableKey = ''

const selectedProvider = computed(() => providers.value.find((provider) => provider.key === providerKey.value))
const nextSlot = computed(() => {
  const slots = draft.value?.samples.map((sample) => sample.slot) ?? []
  return slots.length ? Math.max(...slots) + 1 : 1
})
const canRecord = computed(() => Boolean(draft.value) && nextSlot.value <= limits.value.maxSamples && !uploading.value)
const canValidate = computed(() => (draft.value?.samples.length ?? 0) >= limits.value.minSamples && draft.value?.validation.status !== 'inconsistent' && !uploading.value && !validating.value)

function newSpeakerKey() {
  return `spk_${crypto.randomUUID().replaceAll('-', '')}`
}

async function load() {
  loading.value = true
  error.value = ''
  try {
    const [page, summary] = await Promise.all([
      providersApi.list({ type: 'speaker', pageSize: 100 }),
      speakersApi.summary(),
    ])
    providers.value = page.items.filter((provider) => provider.enabled === 1)
    if (providers.value.length === 1) providerKey.value = providers.value[0].key
    limits.value = {
      minClipMs: summary.enrollment.min_clip_ms,
      maxClipMs: summary.enrollment.max_clip_ms,
      minSamples: summary.enrollment.min_samples,
      maxSamples: summary.enrollment.max_samples,
    }
    if (props.speakerKey) {
      speaker.value = await speakersApi.get(props.speakerKey)
      const openDraft = speaker.value.enrollment_drafts[0]
      if (openDraft) draft.value = await speakersApi.getDraft(speaker.value.key, openDraft.id)
    }
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    loading.value = false
  }
}

async function startEnrollment() {
  if (!selectedProvider.value || saving.value) return
  saving.value = true
  error.value = ''
  try {
    if (!speaker.value) {
      stableKey ||= newSpeakerKey()
      try {
        speaker.value = await speakersApi.create({ key: stableKey, name: name.value.trim(), description: description.value.trim() || undefined })
      } catch (cause) {
        // A dropped create response can leave the stable-key profile committed.
        speaker.value = await speakersApi.get(stableKey).catch(() => { throw cause })
      }
    }
    draft.value = await speakersApi.createDraft(
      speaker.value.key,
      { provider_key: selectedProvider.value.key, expected_provider_revision: selectedProvider.value.revision },
      speaker.value.revision,
    )
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

async function refreshDraft() {
  if (!draft.value || !speaker.value) return
  try { draft.value = await speakersApi.getDraft(speaker.value.key, draft.value.id) } catch { /* keep the last safe revision */ }
}

async function startRecording() {
  if (!canRecord.value) return
  error.value = ''
  await recorder.start(limits.value, () => void stopRecording())
  if (recorder.error.value) error.value = 'Không thể truy cập micro. Hãy cấp quyền rồi thử lại.'
}

async function stopRecording() {
  const wav = await recorder.stop()
  if (!wav || !draft.value || !speaker.value) return
  uploading.value = true
  const controller = new AbortController()
  uploadAbort?.abort()
  uploadAbort = controller
  try {
    draft.value = await speakersApi.uploadSample(speaker.value.key, draft.value.id, nextSlot.value, wav, draft.value.revision, controller.signal)
  } catch (cause) {
    if (!controller.signal.aborted) { error.value = formatApiError(cause); await refreshDraft() }
  } finally {
    if (uploadAbort === controller) uploadAbort = undefined
    uploading.value = false
  }
}

async function removeSample(slot: number) {
  if (!draft.value || !speaker.value) return
  uploading.value = true
  try {
    draft.value = await speakersApi.deleteSample(speaker.value.key, draft.value.id, slot, draft.value.revision)
  } catch (cause) {
    error.value = formatApiError(cause)
    await refreshDraft()
  } finally { uploading.value = false }
}

async function startHoldout() {
  if (!canValidate.value) return
  error.value = ''
  await holdoutRecorder.start(limits.value, () => void stopHoldout())
  if (holdoutRecorder.error.value) error.value = 'Không thể truy cập micro. Hãy cấp quyền rồi thử lại.'
}

async function stopHoldout() {
  const wav = await holdoutRecorder.stop()
  if (!wav || !draft.value || !speaker.value) return
  validating.value = true
  const controller = new AbortController()
  operationAbort?.abort()
  operationAbort = controller
  try {
    const result = await speakersApi.validateHoldout(speaker.value.key, draft.value.id, wav, draft.value.revision, controller.signal)
    draft.value = result.enrollment
    if (result.validation.status === 'passed' && result.validation.valid_for_current_revision) {
      const finalized = await speakersApi.finalizeDraft(speaker.value.key, result.enrollment.id, speaker.value.revision, result.enrollment.revision, controller.signal)
      speaker.value = finalized.speaker
      draft.value = undefined
      completed.value = true
      emit('completed', finalized.speaker)
    }
  } catch (cause) {
    error.value = formatApiError(cause)
    await refreshDraft()
  } finally { if (operationAbort === controller) operationAbort = undefined; validating.value = false }
}

async function cancelDraft() {
  if (!draft.value || !speaker.value) return
  saving.value = true
  try {
    await speakersApi.cancelDraft(speaker.value.key, draft.value.id, draft.value.revision)
    draft.value = undefined
  } catch (cause) { error.value = formatApiError(cause) } finally { saving.value = false }
}

function close() {
  if (draft.value) { error.value = 'Chọn “Dừng và tiếp tục sau” hoặc “Hủy bản nháp” để bảo toàn trạng thái đăng ký.'; return }
  emit('update:open', false)
}
function pause() { emit('update:open', false) }
function reset() {
  uploadAbort?.abort(); operationAbort?.abort(); uploadAbort = undefined; operationAbort = undefined
  recorder.dispose(); holdoutRecorder.dispose()
  mode.value = props.speakerKey ? 'validated' : props.defaultMode
  providerKey.value = ''; name.value = ''; description.value = ''; speaker.value = undefined; draft.value = undefined
  error.value = ''; completed.value = false; stableKey = ''
}

watch(() => props.open, (open) => { reset(); if (open) void load() })
onBeforeUnmount(reset)
</script>

<template>
  <QuickSpeakerEnrollment v-if="open && mode === 'quick' && !speakerKey" :open="open" @update:open="emit('update:open', $event)" @created="emit('completed', $event)" />
  <BaseModal v-else :model-value="open" title="Thêm người nói" description="Xác nhận mẫu giọng nói không đồng nghĩa với xác thực danh tính." @update:model-value="close">
    <div class="space-y-4">
      <p v-if="error" role="alert" class="rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">{{ error }}</p>
      <div v-if="loading" class="text-sm text-muted-foreground">Đang tải…</div>
      <template v-else-if="completed && speaker">
        <CheckCircle class="mx-auto size-10 text-emerald-600" />
        <p class="text-center font-medium">Mẫu giọng nói đã được xác nhận</p>
        <p class="text-center text-sm text-muted-foreground">{{ speaker.name }} · {{ limits.minSamples }}+ mẫu và một mẫu đối chiếu</p>
      </template>
      <template v-else-if="!draft">
        <div v-if="!speakerKey" class="space-y-2 rounded-md border p-3 text-sm">
          <label class="flex items-center gap-2"><input v-model="mode" type="radio" value="validated" /> Xác nhận giọng nói (khuyến nghị)</label>
          <label class="flex items-center gap-2"><input v-model="mode" type="radio" value="quick" /> Quick enrollment — 1 sample, not fully confirmed</label>
        </div>
        <template v-if="mode === 'validated'">
          <label class="block space-y-1"><span class="text-sm font-medium">Speaker Provider</span><select v-model="providerKey" class="admin-input"><option value="" disabled>Chọn Provider</option><option v-for="provider in providers" :key="provider.key" :value="provider.key">{{ provider.name }} ({{ provider.key }}@{{ provider.revision }})</option></select></label>
          <template v-if="!speakerKey"><label class="block space-y-1"><span class="text-sm font-medium">Tên</span><input v-model="name" class="admin-input" maxlength="128" /></label><label class="block space-y-1"><span class="text-sm font-medium">Mô tả</span><textarea v-model="description" class="admin-input" rows="3" maxlength="2048" /></label></template>
          <p v-if="speakerKey" class="text-sm text-muted-foreground">Ghi bộ mẫu mới để xác nhận người nói hiện có. Mẫu Quick không được dùng lại.</p>
        </template>
      </template>
      <template v-else>
        <p class="text-sm">{{ draft.samples.length }}/{{ limits.minSamples }} mẫu đã được server chấp nhận. Mỗi mẫu được kiểm tra chất lượng trước khi tiếp tục.</p>
        <ul class="space-y-1 text-sm"><li v-for="sample in draft.samples" :key="sample.slot" class="flex items-center justify-between"><span>Mẫu {{ sample.slot }} · {{ (sample.duration_ms / 1000).toFixed(1) }}s · hợp lệ</span><Button variant="ghost" size="sm" :disabled="uploading || validating" @click="removeSample(sample.slot)"><Trash2 class="size-4" /></Button></li></ul>
        <p v-if="recorder.recording.value || holdoutRecorder.recording.value" class="text-sm text-muted-foreground">Đang ghi {{ ((recorder.recording.value ? recorder.elapsedMs.value : holdoutRecorder.elapsedMs.value) / 1000).toFixed(1) }}s</p>
        <p v-else class="text-sm text-muted-foreground">Nói tự nhiên {{ limits.minClipMs / 1000 }}–{{ limits.maxClipMs / 1000 }} giây. Mẫu đối chiếu phải là lần thu mới.</p>
        <p v-if="draft.validation.status !== 'none'" class="text-sm" :class="draft.validation.status === 'passed' ? 'text-emerald-600' : 'text-destructive'">Kết quả: {{ draft.validation.status === 'passed' ? 'Đã xác nhận nhất quán' : draft.validation.status === 'inconsistent' ? 'Các mẫu chưa nhất quán; hãy thay mẫu rồi thu đối chiếu mới.' : 'Mẫu đối chiếu chưa đạt; hãy thu lại.' }}</p>
        <div class="flex flex-wrap gap-2">
          <Button v-if="recorder.recording.value" variant="outline" @click="stopRecording"><Square class="size-4" />Dừng ghi mẫu</Button>
          <Button v-else-if="nextSlot <= limits.maxSamples" :disabled="!canRecord" @click="startRecording"><Mic class="size-4" />{{ uploading ? 'Đang tải…' : 'Ghi mẫu' }}</Button>
          <Button v-if="holdoutRecorder.recording.value" variant="outline" @click="stopHoldout"><Square class="size-4" />Dừng mẫu đối chiếu</Button>
          <Button v-else :disabled="!canValidate" @click="startHoldout"><Mic class="size-4" />{{ validating ? 'Đang kiểm tra…' : 'Thu mẫu xác nhận' }}</Button>
        </div>
      </template>
    </div>
    <template #footer><div class="flex justify-end gap-2"><Button v-if="completed" @click="close">Mở chi tiết người nói</Button><template v-else-if="!draft"><Button variant="outline" @click="close">Hủy</Button><Button v-if="mode === 'validated'" :disabled="saving || !selectedProvider || (!speakerKey && !name.trim())" @click="startEnrollment">{{ saving ? 'Đang tạo…' : 'Tiếp tục' }}</Button></template><template v-else><Button variant="outline" @click="pause">Dừng và tiếp tục sau</Button><Button variant="outline" :disabled="saving" @click="cancelDraft"><X class="size-4" />Hủy bản nháp</Button></template></div></template>
  </BaseModal>
</template>

<script setup lang="ts">
import { CheckCircle, Mic, Square } from '@lucide/vue'
import { computed, onBeforeUnmount, ref, watch } from 'vue'

import { formatApiError } from '@/api/errors'
import { providersApi } from '@/api/providers'
import { speakersApi } from '@/api/speakers'
import type { AdminProvider } from '@/api/types/providers'
import type { Speaker, SpeakerCapture } from '@/api/types/speakers'
import BaseModal from '@/components/admin/BaseModal.vue'
import { Button } from '@/components/ui/button'
import { useMicrophoneRecorder } from '@/composables/useMicrophoneRecorder'

const open = defineModel<boolean>('open', { required: true })
const emit = defineEmits<{ created: [Speaker] }>()
const recorder = useMicrophoneRecorder()
const providers = ref<AdminProvider[]>([])
const providerKey = ref('')
const limits = ref({ minClipMs: 5_000, maxClipMs: 10_000 })
const step = ref<'record' | 'result' | 'details'>('record')
const capture = ref<SpeakerCapture>()
const error = ref('')
const loading = ref(false)
const available = ref(true)
const uploading = ref(false)
const saving = ref(false)
const name = ref('')
const description = ref('')
const created = ref<Speaker>()
let controller: AbortController | undefined

const selected = computed(() => providers.value.find((provider) => provider.key === providerKey.value))
const canStop = computed(() => recorder.elapsedMs.value >= limits.value.minClipMs)

async function load() {
  loading.value = true
  error.value = ''
  try {
    const [page, summary] = await Promise.all([providersApi.list({ type: 'speaker', pageSize: 200 }), speakersApi.summary()])
    providers.value = page.items.filter((provider) => provider.enabled === 1)
    limits.value = { minClipMs: summary.enrollment.min_clip_ms, maxClipMs: summary.enrollment.max_clip_ms }
    if (providers.value.length === 1) providerKey.value = providers.value[0].key
    available.value = summary.available
    if (!summary.available) error.value = 'Runtime người nói hiện không khả dụng.'
  } catch (cause) { error.value = formatApiError(cause) } finally { loading.value = false }
}

async function start() {
  if (!selected.value || uploading.value || !available.value) return
  error.value = ''
  await recorder.start(limits.value, () => void stop())
  if (recorder.error.value) error.value = 'Không thể truy cập micro. Hãy cấp quyền trong trình duyệt rồi thử lại.'
}

async function stop() {
  if (uploading.value || !selected.value) return
  const wav = await recorder.stop()
  if (!wav) { error.value = 'Không có dữ liệu âm thanh. Hãy ghi âm lại.'; return }
  uploading.value = true
  error.value = ''
  controller?.abort()
  controller = new AbortController()
  try {
    capture.value = await speakersApi.capture(selected.value.key, wav, selected.value.revision, controller.signal)
    step.value = 'result'
  } catch (cause) {
    if (!controller.signal.aborted) error.value = formatApiError(cause)
  } finally { uploading.value = false }
}

function retry() { capture.value = undefined; step.value = 'record'; error.value = '' }

async function save() {
  if (!capture.value || saving.value || !name.value.trim()) return
  saving.value = true
  error.value = ''
  try {
    const result = await speakersApi.createFromCapture({ capture_id: capture.value.capture_id, name: name.value.trim(), description: description.value.trim() || undefined })
    created.value = result.speaker
    emit('created', result.speaker)
  } catch (cause) {
    error.value = formatApiError(cause)
    if (/capture_(not_found|expired)|runtime_incompatible/.test(error.value)) retry()
  } finally { saving.value = false }
}

function reset() {
  controller?.abort(); controller = undefined; recorder.dispose()
  step.value = 'record'; capture.value = undefined; error.value = ''; name.value = ''; description.value = ''; created.value = undefined
}
watch(open, (value) => { if (value) void load(); else reset() })
onBeforeUnmount(reset)
</script>

<template>
  <BaseModal v-model="open" title="Đăng ký nhanh" description="Lưu một mẫu giọng nói; chưa có mẫu đối chiếu để xác nhận độ ổn định.">
    <div class="space-y-5">
      <div class="flex justify-between text-sm text-muted-foreground"><span :class="step === 'record' ? 'font-medium text-foreground' : ''">1 Ghi âm</span><span :class="step === 'result' ? 'font-medium text-foreground' : ''">2 Kiểm tra</span><span :class="step === 'details' ? 'font-medium text-foreground' : ''">3 Thông tin</span></div>
      <p v-if="error" role="alert" class="rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">{{ error }}</p>
      <template v-if="!created && step === 'record'">
        <label class="block space-y-1"><span class="text-sm font-medium">Speaker Provider</span><select v-model="providerKey" class="admin-input" :disabled="loading || recorder.recording.value || uploading"><option value="" disabled>Chọn Provider</option><option v-for="provider in providers" :key="provider.key" :value="provider.key">{{ provider.name }}</option></select></label>
        <div class="rounded-lg border p-6 text-center"><Mic class="mx-auto size-8" /><p class="mt-3 font-medium">{{ recorder.recording.value ? `Đang ghi âm ${(recorder.elapsedMs.value / 1000).toFixed(1)}s` : 'Sẵn sàng ghi âm' }}</p><p class="mt-1 text-sm text-muted-foreground">Hãy nói tự nhiên {{ limits.minClipMs / 1000 }}–{{ limits.maxClipMs / 1000 }} giây ở nơi ít tiếng ồn.</p></div>
      </template>
      <template v-else-if="!created && step === 'result' && capture"><div class="py-5 text-center"><CheckCircle class="mx-auto size-10 text-emerald-600" /><p class="mt-3 font-medium">Mẫu giọng nói hợp lệ</p><p class="text-sm text-muted-foreground">Thời lượng: {{ (capture.quality.duration_ms / 1000).toFixed(1) }} giây · Có tiếng nói: {{ (capture.quality.speech_ms / 1000).toFixed(1) }} giây</p><p class="mt-3 text-sm">Mẫu đã được xử lý, nhưng danh tính chưa được xác minh.</p></div></template>
      <template v-else-if="!created"><p class="text-sm">Mẫu giọng nói đã sẵn sàng. Sau khi lưu, voiceprint có trạng thái pending và chưa có holdout.</p><label class="block space-y-1"><span class="text-sm font-medium">Tên người nói *</span><input v-model="name" class="admin-input" maxlength="128" required /></label><label class="block space-y-1"><span class="text-sm font-medium">Mô tả (không bắt buộc)</span><textarea v-model="description" class="admin-input" rows="3" maxlength="2048" /></label></template>
      <div v-else class="py-5 text-center"><CheckCircle class="mx-auto size-10 text-emerald-600" /><p class="mt-3 font-medium">Đã đăng ký người nói thành công.</p><p class="text-sm text-muted-foreground">Đã lưu 1 mẫu — chưa xác nhận độ ổn định</p></div>
    </div>
    <template #footer><div class="flex flex-wrap justify-end gap-2"><Button v-if="created" variant="outline" @click="open = false">Đóng</Button><Button v-else-if="step === 'record' && recorder.recording.value" :disabled="!canStop || uploading" @click="stop"><Square class="size-4" />Dừng ghi âm</Button><Button v-else-if="step === 'record'" :disabled="!selected || loading || uploading || !available" @click="start"><Mic class="size-4" />Bắt đầu ghi âm</Button><template v-else-if="step === 'result'"><Button variant="outline" @click="retry">Ghi âm lại</Button><Button @click="step = 'details'">Tiếp tục nhập thông tin</Button></template><template v-else><Button variant="outline" :disabled="saving" @click="step = 'result'">Quay lại</Button><Button :disabled="saving || !name.trim()" @click="save">{{ saving ? 'Đang lưu…' : 'Lưu người nói' }}</Button></template></div></template>
  </BaseModal>
</template>

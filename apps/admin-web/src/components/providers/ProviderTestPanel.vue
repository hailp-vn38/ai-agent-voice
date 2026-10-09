<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { providersApi } from '@/api/providers'
import { providerAdaptersApi } from '@/api/provider-adapters'
import { formatApiError } from '@/api/errors'
import type { ProviderTestDraft } from '@/api/types/providers'
import type { ProviderType } from '@/domain/admin'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useMicrophoneRecorder } from '@/composables/useMicrophoneRecorder'
import VoiceRecordingDock from '@/components/voice/VoiceRecordingDock.vue'

const props = defineProps<{ type: ProviderType; savedKey?: string; revision?: number; adapter?: string; capabilities?: Record<string, unknown>; draft?: ProviderTestDraft }>()
const { t } = useI18n()
const recorder = useMicrophoneRecorder()
const text = ref('Xin chào')
const voice = ref('')
const language = ref('')
const languageResult = ref('')
const durationMs = ref<number>()
const rtf = ref<number>()
const discoveredCapabilities = ref<Record<string, unknown>>()
const capabilities = computed(() => props.capabilities ?? discoveredCapabilities.value)
function options(key: 'voices' | 'languages') {
  const values = capabilities.value?.[key]
  return Array.isArray(values) ? values.flatMap((item) => {
    if (typeof item === 'string') return [{ id: item, name: item }]
    if (item && typeof item === 'object' && typeof item.id === 'string') return [{ id: item.id as string, name: String(item.name ?? item.id) }]
    return []
  }) : []
}
watch(() => [props.adapter, props.capabilities] as const, async ([adapter, provided], _, onCleanup) => {
  discoveredCapabilities.value = undefined
  if (!adapter || provided) return
  const pending = new AbortController()
  onCleanup(() => pending.abort())
  try { const descriptor = await providerAdaptersApi.get(adapter, pending.signal); if (!pending.signal.aborted) discoveredCapabilities.value = descriptor.capabilities } catch { /* Inference remains available without optional selectors. */ }
}, { immediate: true })
const audio = ref<Blob>()
const audioUrl = ref('')
const result = ref('')
const elapsed = ref<number>()
const error = ref('')
const busy = ref(false)
const status = ref<'idle' | 'success' | 'failed' | 'stale'>('idle')
const supported = computed(() => ['asr', 'llm', 'tts', 'vad'].includes(props.type) && (props.type !== 'vad' || Boolean(props.savedKey)))
let controller: AbortController | undefined
function clear() {
  controller?.abort()
  controller = undefined
  busy.value = false
  result.value = ''; error.value = ''; elapsed.value = undefined; languageResult.value = ''; durationMs.value = undefined; rtf.value = undefined; status.value = 'idle'
  if (audioUrl.value) URL.revokeObjectURL(audioUrl.value)
  audioUrl.value = ''
}
function reset() { clear(); audio.value = undefined; recorder.dispose() }
function invalidate() {
  const stale = status.value !== 'idle'
  clear()
  if (stale) status.value = 'stale'
}
watch([text, voice, language], invalidate)
watch(() => [props.savedKey, props.revision, props.type, props.draft], () => {
  invalidate(); audio.value = undefined; voice.value = ''; language.value = ''; recorder.dispose()
}, { deep: true })
onBeforeUnmount(reset)
async function stopRecording() { audio.value = await recorder.stop() }
async function startRecording() {
  clear(); audio.value = undefined
  await recorder.start({ minClipMs: 0, maxClipMs: 30_000 }, () => { void stopRecording() })
}
async function run() {
  clear()
  const pending = new AbortController()
  controller = pending
  busy.value = true
  try {
    if (props.type === 'tts') {
      const response = props.draft
        ? await providersApi.testDraftTts(props.draft, { text: text.value, ...(voice.value ? { voice: voice.value } : {}), ...(language.value ? { language: language.value } : {}) }, pending.signal)
        : await providersApi.testTtsAudio(props.savedKey!, { text: text.value, ...(voice.value ? { voice: voice.value } : {}), ...(language.value ? { language: language.value } : {}) }, pending.signal)
      if (pending.signal.aborted) return
      audioUrl.value = URL.createObjectURL(response.audio)
      elapsed.value = response.elapsedMs
    } else if (props.type === 'vad') {
      const response = await providersApi.testVad(props.savedKey!, pending.signal)
      if (pending.signal.aborted) return
      result.value = JSON.stringify(response)
    } else {
      const response = props.type === 'asr'
        ? props.draft ? await providersApi.testDraftAsr(props.draft, audio.value!, pending.signal) : await providersApi.testAsr(props.savedKey!, { audio: audio.value! }, pending.signal)
        : props.draft ? await providersApi.testDraftLlm(props.draft, { text: text.value, ...(voice.value ? { voice: voice.value } : {}), ...(language.value ? { language: language.value } : {}) }, pending.signal) : await providersApi.testLlm(props.savedKey!, { input: text.value }, pending.signal)
      if (pending.signal.aborted) return
      result.value = response.result.text
      elapsed.value = response.metrics.elapsed_ms
      languageResult.value = response.result.language ?? ''
      durationMs.value = response.metrics.audio_duration_ms
      rtf.value = response.metrics.rtf
    }
    status.value = 'success'
  } catch (cause) {
    if (!pending.signal.aborted) { error.value = formatApiError(cause); status.value = 'failed' }
  } finally { if (controller === pending) busy.value = false }
}
</script>

<template>
  <section class="space-y-3" aria-live="polite">
    <h3 class="text-sm font-semibold">{{ t('diagnostics.lastTest') }}</h3>
    <p v-if="draft" class="text-xs text-muted-foreground">{{ t('diagnostics.draftHint') }}</p>
    <p v-if="!supported" class="text-sm text-muted-foreground">{{ t('diagnostics.unsupported') }}</p>
    <template v-else>
      <label v-if="type === 'llm' || type === 'tts'" class="block space-y-1.5">
        <span class="text-sm">{{ t('diagnostics.text') }}</span>
        <textarea v-model="text" class="admin-textarea min-h-24" :maxlength="type === 'llm' ? 8192 : 4096" :disabled="busy" />
      </label>
      <div v-if="type === 'tts'" class="grid gap-3 sm:grid-cols-2">
        <label v-for="field in (['voices', 'languages'] as const)" v-show="options(field).length" :key="field" class="space-y-1.5 text-sm">
          <span>{{ t(field === 'voices' ? 'diagnostics.voice' : 'diagnostics.language') }}</span>
          <select class="admin-input" :disabled="busy" :value="field === 'voices' ? voice : language" @change="field === 'voices' ? voice = ($event.target as HTMLSelectElement).value : language = ($event.target as HTMLSelectElement).value">
            <option value="">{{ t('diagnostics.configDefault') }}</option><option v-for="option in options(field)" :key="option.id" :value="option.id">{{ option.name }}</option>
          </select>
        </label>
      </div>
      <div v-if="type === 'asr'" class="space-y-2">
        <VoiceRecordingDock
          :recording="recorder.recording.value"
          :paused="recorder.paused.value"
          :starting="recorder.starting.value"
          :elapsed-ms="recorder.elapsedMs.value"
          :level="recorder.level.value"
          :min-clip-ms="0"
          :max-clip-ms="30_000"
          :busy="busy"
          :clip-ready="Boolean(audio)"
          allow-pause
          @start="startRecording"
          @stop="stopRecording"
          @toggle-pause="recorder.togglePause"
        />
        <p v-if="audio" class="text-xs text-muted-foreground">{{ t('diagnostics.clipReady') }}</p>
        <p v-if="recorder.error.value" role="alert" class="text-sm text-danger">{{ recorder.error.value }}</p>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <Button type="button" data-run-test :disabled="busy || recorder.starting.value || recorder.recording.value || (type === 'asr' ? !audio : type !== 'vad' && !text.trim())" @click="run">{{ busy ? t('diagnostics.running') : t('diagnostics.run') }}</Button>
        <Button v-if="busy" type="button" variant="outline" @click="clear">{{ t('diagnostics.cancel') }}</Button>
        <span class="text-xs text-muted-foreground">{{ status === 'idle' ? t('diagnostics.notTested') : status === 'success' ? t('diagnostics.success') : status === 'stale' ? t('diagnostics.stale') : t('diagnostics.failed') }} <template v-if="elapsed !== undefined">· {{ elapsed }} ms</template></span>
      </div>
      <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
      <pre v-if="result" class="max-h-60 overflow-auto whitespace-pre-wrap break-words rounded-lg border p-3 text-sm" :aria-label="t('diagnostics.result')">{{ result }}</pre>
      <p v-if="languageResult || durationMs !== undefined" class="text-xs text-muted-foreground">
        <span v-if="languageResult">{{ t('diagnostics.language') }}: {{ languageResult }} · </span>
        <span v-if="durationMs !== undefined">{{ t('diagnostics.duration') }}: {{ (durationMs / 1000).toFixed(2) }} s</span>
        <span v-if="rtf !== undefined"> · RTF: {{ rtf.toFixed(3) }}</span>
      </p>
      <audio v-if="audioUrl" :src="audioUrl" controls class="w-full" @loadedmetadata="durationMs = Number.isFinite(($event.target as HTMLAudioElement).duration) ? ($event.target as HTMLAudioElement).duration * 1000 : undefined" />
    </template>
  </section>
</template>

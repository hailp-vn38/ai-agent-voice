<script setup lang="ts">
import { computed } from 'vue'
import { LoaderCircle, Mic, Pause, Play, Square } from '@lucide/vue'

/**
 * Shared recording control for Provider ASR diagnostics and Speaker enrollment.
 * UI only: the owning flow controls microphone capture, WAV encoding and upload.
 */
const props = withDefaults(defineProps<{
  recording: boolean
  elapsedMs: number
  level: number
  minClipMs?: number
  maxClipMs?: number
  starting?: boolean
  busy?: boolean
  disabled?: boolean
  paused?: boolean
  allowPause?: boolean
  clipReady?: boolean
}>(), {
  minClipMs: 0,
  maxClipMs: 30_000,
  starting: false,
  busy: false,
  disabled: false,
  paused: false,
  allowPause: false,
  clipReady: false,
})

const emit = defineEmits<{ start: []; stop: []; 'toggle-pause': [] }>()
const bars = Array.from({ length: 36 }, (_, index) => index)
const normalizedLevel = computed(() => Number.isFinite(props.level) ? Math.max(0, Math.min(1, props.level)) : 0)
const progress = computed(() => Math.min(100, Math.max(0, props.elapsedMs) / Math.max(1, props.maxClipMs) * 100))
const canStop = computed(() => props.recording && !props.busy && !props.starting && !props.disabled && props.elapsedMs >= props.minClipMs)
const remainingSeconds = computed(() => Math.max(0, Math.ceil((props.minClipMs - props.elapsedMs) / 1000)))
const timer = computed(() => {
  const seconds = Math.floor(Math.max(0, props.elapsedMs) / 1000)
  return `${String(Math.floor(seconds / 60)).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}`
})
const status = computed(() => {
  if (props.starting) return 'Đang mở microphone…'
  if (props.busy) return 'Đang xử lý âm thanh…'
  if (props.paused && props.recording) return 'Đã tạm dừng'
  if (props.recording) return 'Đang ghi âm'
  return props.clipReady ? 'Đã ghi âm xong' : 'Sẵn sàng ghi âm'
})
const hint = computed(() => {
  if (props.starting) return 'Vui lòng cấp quyền sử dụng microphone nếu được yêu cầu.'
  if (props.busy) return 'Giữ nguyên trang cho đến khi xử lý hoàn tất.'
  if (props.paused && props.recording) return 'Nhấn tiếp tục để ghi thêm giọng nói.'
  if (props.recording) {
    if (remainingSeconds.value) return `Cần ghi thêm ${remainingSeconds.value} giây trước khi có thể dừng.`
    return 'Nhấn dừng khi bạn nói xong.'
  }
  if (props.clipReady) return 'Mẫu ghi âm đã sẵn sàng để kiểm tra.'
  if (props.minClipMs) return `Nói tự nhiên ${props.minClipMs / 1000}–${props.maxClipMs / 1000} giây ở nơi ít tiếng ồn.`
  return `Nói tự nhiên, tối đa ${props.maxClipMs / 1000} giây.`
})

// Keep the bars flat in silence; only real microphone RMS drives their heights.
function barHeight(index: number) {
  const shape = 0.22 + 0.78 * Math.abs(Math.sin((index + 2) * 1.07) * Math.cos(index * 0.47))
  return `${Math.round(4 + (props.recording && !props.paused ? normalizedLevel.value : 0) * 43 * shape)}px`
}

function action() {
  if (props.starting || props.busy || props.disabled) return
  if (props.recording) {
    if (canStop.value) emit('stop')
  } else {
    emit('start')
  }
}
</script>

<template>
  <div
    class="voice-recording-dock rounded-xl border border-border bg-card p-4 text-foreground sm:p-5"
    :class="{ 'is-recording': recording && !paused, 'is-paused': paused && recording }"
    data-voice-recording-dock
  >
    <div class="flex items-center gap-3">
      <span class="voice-dock-mic grid size-11 shrink-0 place-items-center rounded-xl" aria-hidden="true">
        <Mic :size="22" :stroke-width="1.8" />
      </span>
      <div class="min-w-0 flex-1">
        <div class="flex items-center gap-2 text-sm font-semibold">
          <span class="voice-dock-dot size-2 shrink-0 rounded-full" aria-hidden="true" />
          <span aria-live="polite">{{ status }}</span>
        </div>
        <p class="mt-1 text-xs leading-relaxed text-muted-foreground">{{ hint }}</p>
      </div>
      <span class="shrink-0 text-lg font-semibold tabular-nums tracking-tight sm:text-xl" role="timer" :aria-label="`Thời gian ghi âm: ${timer}`">{{ timer }}</span>
    </div>

    <div class="voice-dock-wave mt-3 flex h-16 items-center justify-center gap-[3px] overflow-hidden rounded-lg px-2" aria-hidden="true">
      <span
        v-for="index in bars"
        :key="index"
        class="voice-dock-bar min-w-[2px] max-w-1 flex-1 rounded-full"
        :style="{ height: barHeight(index) }"
      />
    </div>

    <div class="mt-3 flex flex-wrap items-center gap-3">
      <div
        class="voice-dock-progress h-1 min-w-16 flex-1 overflow-hidden rounded-full"
        role="progressbar"
        :aria-valuemin="0"
        :aria-valuemax="maxClipMs"
        :aria-valuenow="Math.min(maxClipMs, Math.max(0, elapsedMs))"
        aria-label="Tiến trình ghi âm"
      >
        <div class="voice-dock-progress-fill h-full rounded-full" :style="{ width: `${progress}%` }" />
      </div>

      <button
        v-if="allowPause && recording"
        type="button"
        class="voice-dock-secondary inline-flex min-h-10 items-center justify-center gap-2 rounded-lg border border-border px-3 text-xs font-semibold hover:bg-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-50"
        :disabled="starting || busy || disabled"
        @click="emit('toggle-pause')"
      >
        <Play v-if="paused" :size="15" />
        <Pause v-else :size="15" />
        {{ paused ? 'Tiếp tục' : 'Tạm dừng' }}
      </button>

      <button
        type="button"
        class="voice-dock-action inline-flex min-h-10 items-center justify-center gap-2 rounded-lg px-4 text-xs font-semibold focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-50"
        :disabled="starting || busy || disabled || (recording && !canStop)"
        @click="action"
      >
        <LoaderCircle v-if="starting || busy" :size="16" class="voice-dock-spinner" aria-hidden="true" />
        <Square v-else-if="recording" :size="14" fill="currentColor" aria-hidden="true" />
        <Mic v-else :size="16" aria-hidden="true" />
        {{ starting ? 'Đang mở mic…' : busy ? 'Đang xử lý…' : recording ? 'Dừng ghi âm' : 'Bắt đầu ghi âm' }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.voice-recording-dock { --voice-violet: var(--studio-violet); --voice-cyan: var(--studio-cyan); }
.voice-dock-mic { color: var(--voice-violet); background: color-mix(in oklch, var(--voice-violet) 13%, var(--card)); }
.voice-dock-dot { background: var(--muted-foreground); }
.is-recording .voice-dock-dot { background: var(--danger); box-shadow: 0 0 0 4px color-mix(in oklch, var(--danger) 14%, transparent); animation: voice-dot-breathe 1.7s ease-in-out infinite; }
.voice-dock-wave { background: color-mix(in oklch, var(--voice-violet) 5%, var(--card)); }
.voice-dock-bar { background: linear-gradient(to top, var(--voice-violet), var(--voice-cyan)); transition: height 110ms linear; }
.voice-dock-progress { background: color-mix(in oklch, var(--voice-violet) 14%, var(--card)); }
.voice-dock-progress-fill { background: linear-gradient(90deg, var(--voice-violet), var(--voice-cyan)); transition: width 100ms linear; }
.voice-dock-action { background: var(--primary); color: var(--primary-foreground); }
.is-recording .voice-dock-action, .is-paused .voice-dock-action { background: var(--danger); color: white; }
.voice-dock-action:not(:disabled):hover { filter: brightness(1.06); }
.voice-dock-spinner { animation: voice-spin 1s linear infinite; }
@keyframes voice-dot-breathe { 50% { opacity: .55; box-shadow: 0 0 0 7px color-mix(in oklch, var(--danger) 5%, transparent); } }
@keyframes voice-spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) {
  .voice-recording-dock *, .voice-recording-dock *::before, .voice-recording-dock *::after { animation: none !important; transition: none !important; }
}
</style>

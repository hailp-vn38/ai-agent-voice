// Capture one enrollment clip from the microphone and turn it into a PCM16 mono 16 kHz WAV.
//
// Everything the browser owns — MediaStream tracks, AudioContext, the worklet node, timers — is
// released by `dispose()`, which callers run on cancel, navigation, and unmount. Nothing is
// persisted to localStorage/IndexedDB; the only artifact is the returned Blob.

import { onBeforeUnmount, ref, shallowRef } from 'vue'

import { TARGET_SAMPLE_RATE, downmixToMono, encodeWavPcm16, resampleLinear } from '@/lib/wav'

export interface RecorderLimits {
  minClipMs: number
  maxClipMs: number
}

const WORKLET_URL = '/enrollment-worklet.js'
const WORKLET_NAME = 'enrollment-capture'

export function useMicrophoneRecorder() {
  const recording = ref(false)
  const elapsedMs = ref(0)
  const level = ref(0)
  const error = ref('')

  const stream = shallowRef<MediaStream | null>(null)
  const context = shallowRef<AudioContext | null>(null)
  const node = shallowRef<AudioWorkletNode | null>(null)
  const source = shallowRef<MediaStreamAudioSourceNode | null>(null)

  let frames: Float32Array[] = []
  let frameCount = 0
  let startedAt = 0
  let tick: ReturnType<typeof setInterval> | undefined
  let autoStop: ReturnType<typeof setTimeout> | undefined
  let limits: RecorderLimits = { minClipMs: 0, maxClipMs: 10_000 }
  let onAutoStop: (() => void) | undefined

  function append(channels: Float32Array[]) {
    const mono = downmixToMono(channels, channels.length)
    const inputRate = context.value?.sampleRate ?? TARGET_SAMPLE_RATE
    const remaining = Math.floor((limits.maxClipMs * inputRate) / 1000) - frameCount
    if (remaining <= 0) return
    const slice = mono.length > remaining ? mono.subarray(0, remaining) : mono
    frames.push(slice.slice())
    frameCount += slice.length
    let sum = 0
    for (const sample of slice) sum += sample * sample
    level.value = Math.min(1, Math.sqrt(sum / Math.max(1, slice.length)) * 4)
  }

  async function start(config: RecorderLimits, onStop?: () => void) {
    error.value = ''
    limits = config
    onAutoStop = onStop
    frames = []
    frameCount = 0
    elapsedMs.value = 0
    level.value = 0
    try {
      const media = await navigator.mediaDevices.getUserMedia({
        audio: {
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
      })
      stream.value = media
      const audio = new AudioContext()
      context.value = audio
      await audio.audioWorklet.addModule(WORKLET_URL)
      const capture = new AudioWorkletNode(audio, WORKLET_NAME)
      node.value = capture
      capture.port.onmessage = (event: MessageEvent<Float32Array[]>) => append(event.data)
      const input = audio.createMediaStreamSource(media)
      source.value = input
      input.connect(capture)
      // Keep the graph alive without routing the microphone to the speakers.
      const mute = audio.createGain()
      mute.gain.value = 0
      capture.connect(mute).connect(audio.destination)

      startedAt = performance.now()
      recording.value = true
      tick = setInterval(() => {
        elapsedMs.value = Math.round(performance.now() - startedAt)
      }, 100)
      autoStop = setTimeout(() => onAutoStop?.(), limits.maxClipMs)
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : 'microphone_unavailable'
      dispose()
    }
  }

  /** Stop capture and return the encoded clip, or `undefined` if nothing usable was captured. */
  async function stop(): Promise<Blob | undefined> {
    const rate = context.value?.sampleRate ?? TARGET_SAMPLE_RATE
    dispose()
    if (frameCount === 0) return undefined
    const total = new Float32Array(frameCount)
    let offset = 0
    for (const frame of frames) {
      total.set(frame, offset)
      offset += frame.length
    }
    frames = []
    const mono = resampleLinear(total, rate, TARGET_SAMPLE_RATE)
    return encodeWavPcm16(mono, TARGET_SAMPLE_RATE)
  }

  function dispose() {
    recording.value = false
    if (tick !== undefined) clearInterval(tick)
    if (autoStop !== undefined) clearTimeout(autoStop)
    tick = undefined
    autoStop = undefined
    if (node.value) {
      node.value.port.onmessage = null
      node.value.disconnect()
      node.value = null
    }
    source.value?.disconnect()
    source.value = null
    stream.value?.getTracks().forEach((track) => track.stop())
    stream.value = null
    void context.value?.close().catch(() => undefined)
    context.value = null
  }

  onBeforeUnmount(dispose)

  return { recording, elapsedMs, level, error, start, stop, dispose }
}

import { mount } from '@vue/test-utils'
import { defineComponent } from 'vue'
import { afterEach, expect, it, vi } from 'vitest'
import { useMicrophoneRecorder } from './useMicrophoneRecorder'
afterEach(() => vi.unstubAllGlobals())
it('releases a microphone granted after cancellation and refuses concurrent starts', async () => {
  let resolve!: (stream: MediaStream) => void
  const getUserMedia = vi.fn(() => new Promise<MediaStream>((done) => { resolve = done }))
  vi.stubGlobal('navigator', { mediaDevices: { getUserMedia } })
  let recorder!: ReturnType<typeof useMicrophoneRecorder>
  const wrapper = mount(defineComponent({ setup() { recorder = useMicrophoneRecorder(); return () => null } }))
  const pending = recorder.start({ minClipMs: 0, maxClipMs: 30000 })
  void recorder.start({ minClipMs: 0, maxClipMs: 30000 })
  expect(getUserMedia).toHaveBeenCalledTimes(1)
  recorder.dispose()
  const stop = vi.fn()
  resolve({ getTracks: () => [{ stop }] } as unknown as MediaStream)
  await pending
  expect(stop).toHaveBeenCalledTimes(1)
  expect(recorder.recording.value).toBe(false)
  wrapper.unmount()
})
it('handles denied microphone permission without keeping capture active', async () => {
  vi.stubGlobal('navigator', { mediaDevices: { getUserMedia: vi.fn().mockRejectedValue(new Error('permission_denied')) } })
  let recorder!: ReturnType<typeof useMicrophoneRecorder>
  const wrapper = mount(defineComponent({ setup() { recorder = useMicrophoneRecorder(); return () => null } }))
  await recorder.start({ minClipMs: 0, maxClipMs: 30000 })
  expect(recorder.recording.value).toBe(false)
  expect(recorder.error.value).toBe('permission_denied')
  wrapper.unmount()
})
it('pauses capture, resumes PCM16 encoding, and releases the audio graph', async () => {
  const trackStop = vi.fn()
  const close = vi.fn().mockResolvedValue(undefined)
  const disconnect = vi.fn()
  let port!: { onmessage: ((event: { data: Float32Array[] }) => void) | null }
  vi.stubGlobal('navigator', { mediaDevices: { getUserMedia: vi.fn().mockResolvedValue({ getTracks: () => [{ stop: trackStop }] }) } })
  vi.stubGlobal('AudioContext', class {
    sampleRate = 48000
    destination = {}
    audioWorklet = { addModule: vi.fn().mockResolvedValue(undefined) }
    close = close
    createMediaStreamSource() { return { connect: vi.fn(), disconnect } }
    createGain() { return { gain: { value: 1 }, connect: vi.fn() } }
  })
  vi.stubGlobal('AudioWorkletNode', class {
    port = port = { onmessage: null }
    disconnect = disconnect
    connect(target: unknown) { return target }
  })
  let recorder!: ReturnType<typeof useMicrophoneRecorder>
  const wrapper = mount(defineComponent({ setup() { recorder = useMicrophoneRecorder(); return () => null } }))
  await recorder.start({ minClipMs: 0, maxClipMs: 30000 })
  port.onmessage!({ data: [new Float32Array(480).fill(0.5)] })
  recorder.togglePause()
  expect(recorder.paused.value).toBe(true)
  port.onmessage!({ data: [new Float32Array(480).fill(1)] })
  recorder.togglePause()
  port.onmessage!({ data: [new Float32Array(480).fill(0.5)] })
  const blob = await recorder.stop()
  const bytes = await new Promise<ArrayBuffer>((resolve) => { const reader = new FileReader(); reader.onload = () => resolve(reader.result as ArrayBuffer); reader.readAsArrayBuffer(blob!) })
  const wav = new DataView(bytes)
  expect(wav.getUint16(22, true)).toBe(1)
  expect(wav.getUint32(24, true)).toBe(16000)
  expect(wav.getUint16(34, true)).toBe(16)
  expect(wav.getUint32(40, true)).toBe(640)
  expect(trackStop).toHaveBeenCalledTimes(1)
  expect(close).toHaveBeenCalledTimes(1)
  expect(port.onmessage).toBeNull()
  wrapper.unmount()
})

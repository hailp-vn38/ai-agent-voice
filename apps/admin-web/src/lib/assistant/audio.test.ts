import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import worklet from './capture-worklet.js?raw'
import { AssistantAudio, isCanonicalOpusPacket } from './audio'
import zerottsArrivals from './fixtures/zerotts-arrivals.json'

function decoded(value: number) {
  return { numberOfFrames: 1440, sampleRate: 24000, close: vi.fn(),
    copyTo: (pcm: Float32Array) => pcm.fill(value) }
}
class PlaybackSource {
  buffer?: { duration: number; pcm: Float32Array }
  onended: (() => void) | null = null
  startTime = 0
  ended = false
  connect = vi.fn()
  disconnect = vi.fn()
  start(time: number) { this.startTime = time }
  stop() { this.ended = true }
}
class PlaybackContext {
  currentTime = 0
  destination = {}
  sources: PlaybackSource[] = []
  gain = { cancelAndHoldAtTime: vi.fn(), setValueAtTime: vi.fn(), linearRampToValueAtTime: vi.fn() }
  createGain() { return { gain: this.gain, connect: vi.fn(), disconnect: vi.fn() } }
  createBuffer(_channels: number, frames: number, rate: number) {
    const pcm = new Float32Array(frames)
    return { duration: frames / rate, pcm, copyToChannel: (data: Float32Array) => pcm.set(data) }
  }
  createBufferSource() { const source = new PlaybackSource(); this.sources.push(source); return source }
  resume = vi.fn(async () => {})
  close = vi.fn(async () => {})
  advance(time: number) {
    this.currentTime = time
    for (const source of this.sources) {
      if (!source.ended && source.startTime + source.buffer!.duration <= time) {
        source.ended = true
        source.onended?.()
      }
    }
  }
}
class PlaybackDecoder {
  static current: PlaybackDecoder
  state = 'configured'
  decodeQueueSize = 0
  value = 0
  flush = vi.fn(async () => {})
  constructor(readonly callbacks: { output: (data: ReturnType<typeof decoded>) => void; error: () => void }) { PlaybackDecoder.current = this }
  configure() {}
  decode() { this.callbacks.output(decoded(++this.value / 1000)) }
  close() { this.state = 'closed' }
  static isConfigSupported = async () => ({ supported: true })
}

describe('buffered speaker playback', () => {
  let context: PlaybackContext
  let audio: AssistantAudio
  let fail = vi.fn<(code: string) => void>()
  const packet = new Uint8Array([24, 0]).buffer
  beforeEach(async () => {
    context = new PlaybackContext()
    vi.stubGlobal('AudioContext', class { constructor() { return context } })
    vi.stubGlobal('AudioDecoder', PlaybackDecoder)
    vi.stubGlobal('EncodedAudioChunk', class {})
    fail = vi.fn<(code: string) => void>()
    audio = new AssistantAudio(fail)
    await audio.enableSpeaker()
    audio.beginPlayback()
  })
  afterEach(() => { audio.disconnect(); vi.unstubAllGlobals() })

  it('buffers delayed chunks and preserves every decoded frame without a mid-sentence gap', () => {
    for (const arrival of zerottsArrivals) {
      context.advance(arrival)
      audio.receive(packet)
    }
    expect(context.sources).toHaveLength(135)
    for (let index = 1; index < context.sources.length; index++) {
      expect(context.sources[index]!.startTime).toBeCloseTo(context.sources[index - 1]!.startTime + 0.060)
    }
    expect(context.sources.map(source => source.buffer!.pcm[0])).toEqual(
      Array.from({ length: 135 }, (_, index) => Math.fround((index + 1) / 1000)),
    )
    expect(fail).not.toHaveBeenCalled()
  })

  it('flushes a short final buffer and waits until it has actually played', async () => {
    audio.receive(packet)
    expect(context.sources).toHaveLength(0)
    const drained = vi.fn()
    audio.finishPlayback(drained)
    await Promise.resolve()
    expect(context.sources).toHaveLength(1)
    expect(drained).not.toHaveBeenCalled()
    context.advance(1)
    expect(drained).toHaveBeenCalledOnce()
  })

  it('rebuilds the buffer after starvation and fades the stop and restart', () => {
    for (let index = 0; index < 22; index++) audio.receive(packet)
    expect(context.sources).toHaveLength(22)
    context.advance(3)
    const fade = context.gain.linearRampToValueAtTime
    expect(fade).toHaveBeenCalledWith(0, expect.any(Number))
    audio.receive(packet)
    expect(context.sources).toHaveLength(22)
    for (let index = 0; index < 21; index++) audio.receive(packet)
    expect(context.sources).toHaveLength(44)
    expect(context.sources[22]!.startTime).toBeCloseTo(3.02)
    expect(context.gain.setValueAtTime).toHaveBeenCalledWith(0, 3.02)
    expect(fade).toHaveBeenCalledWith(1, 3.028)
    expect(fail).not.toHaveBeenCalled()
  })

  it('drops buffered and late decoded audio on cancellation', async () => {
    const old = PlaybackDecoder.current
    audio.receive(packet)
    const drained = vi.fn()
    audio.finishPlayback(drained)
    audio.cancelPlayback()
    old.callbacks.output(decoded(0.5))
    await Promise.resolve()
    expect(context.sources).toHaveLength(0)
    expect(drained).not.toHaveBeenCalled()
    audio.beginPlayback()
    old.callbacks.error()
    expect(audio.muted).toBe(false)
    expect(fail).not.toHaveBeenCalled()
  })

  it('keeps queued playback bounded and releases the final drain on mute', async () => {
    for (let index = 0; index < 33; index++) audio.receive(packet)
    const drained = vi.fn()
    audio.finishPlayback(drained)
    audio.receive(packet)
    await Promise.resolve()
    expect(audio.muted).toBe(true)
    expect(fail).toHaveBeenCalledWith('audio_backpressure')
    expect(drained).toHaveBeenCalledOnce()
    expect(context.sources.every(source => source.ended)).toBe(true)
  })
})

describe('canonical 60 ms Opus packets', () => {
  it('accepts SILK 60 ms and three CELT 20 ms frames, rejects 20 ms and truncated packets', () => {
    expect(isCanonicalOpusPacket(new Uint8Array([24, 1]).buffer)).toBe(true)
    expect(isCanonicalOpusPacket(new Uint8Array([155, 3, 1, 2, 3]).buffer)).toBe(true)
    expect(isCanonicalOpusPacket(new Uint8Array([152, 1]).buffer)).toBe(false)
    expect(isCanonicalOpusPacket(new Uint8Array([155]).buffer)).toBe(false)
    expect(isCanonicalOpusPacket(new ArrayBuffer(0))).toBe(false)
    expect(isCanonicalOpusPacket(new ArrayBuffer(4097))).toBe(false)
  })
})

it('AudioWorklet downmixes hardware channels into exactly 960 sample frames without dropping remainder', () => {
  const output: Float32Array[] = []
  const processor = { port: { postMessage: (frame: Float32Array) => output.push(frame) } }
  class Base { port = processor.port }
  let Capture: new () => { process: (inputs: Float32Array[][]) => boolean } = class { process() { return false } }
  const register = vi.fn((_name: string, ctor: typeof Capture) => { Capture = ctor })
  const source = worklet
  new Function('AudioWorkletProcessor', 'registerProcessor', source)(Base, register)
  const capture = new Capture()
  for (let block = 0; block < 16; block++) capture.process([[new Float32Array(128).fill(0.75), new Float32Array(128).fill(0.25)]])
  expect(output).toHaveLength(2)
  expect(output[0]!.length).toBe(960)
  expect(output[1]![959]).toBe(0.5)
})

it('an old worklet failure cannot cancel a newer microphone capture', async () => {
  const failures: ((error: Error) => void)[] = []
  const tracks = [vi.fn(), vi.fn()]
  let index = 0
  class Context {
    sampleRate = 16000
    audioWorklet = { addModule: () => new Promise<void>((_resolve, reject) => failures.push(reject)) }
    close = vi.fn(async () => {})
  }
  vi.spyOn(AssistantAudio, 'microphoneSupported').mockReturnValue(true)
  vi.stubGlobal('AudioEncoder', { isConfigSupported: async () => ({ supported: true }) })
  vi.stubGlobal('AudioContext', Context)
  vi.stubGlobal('navigator', { mediaDevices: { getUserMedia: async () => {
    const stop = tracks[index++]!
    return { getTracks: () => [{ stop }] }
  } } })
  const audio = new AssistantAudio(() => {})
  try {
    const old = audio.startMicrophone(() => {})
    await vi.waitFor(() => expect(failures).toHaveLength(1))
    audio.cancelMicrophone()
    const next = audio.startMicrophone(() => {})
    await vi.waitFor(() => expect(failures).toHaveLength(2))
    failures[0]!(new Error('old worklet failed'))
    await expect(old).rejects.toThrow('old worklet failed')
    expect(tracks[1]).not.toHaveBeenCalled()
    failures[1]!(new Error('new worklet failed'))
    await expect(next).rejects.toThrow('new worklet failed')
    expect(tracks[1]).toHaveBeenCalledOnce()
  } finally { audio.disconnect(); vi.restoreAllMocks(); vi.unstubAllGlobals() }
})

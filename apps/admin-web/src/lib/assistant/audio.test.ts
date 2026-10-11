import { describe, expect, it, vi } from 'vitest'
import worklet from './capture-worklet.js?raw'
import { AssistantAudio, isCanonicalOpusPacket } from './audio'

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

// Pure browser-side audio encoding for enrollment clips: downmix, resample to the pinned
// 16 kHz, clamp, and serialize PCM16 little-endian WAV. Kept free of Web Audio types so it is
// unit-testable under jsdom.

export const TARGET_SAMPLE_RATE = 16_000
export const TARGET_CHANNELS = 1
export const BITS_PER_SAMPLE = 16

/** Average interleaved channels into one mono channel. A single channel is returned as-is. */
export function downmixToMono(channels: Float32Array[], channelCount: number): Float32Array {
  if (channelCount <= 1 || channels.length <= 1) return channels[0] ?? new Float32Array(0)
  const frames = channels[0].length
  const mono = new Float32Array(frames)
  for (let frame = 0; frame < frames; frame += 1) {
    let sum = 0
    for (let channel = 0; channel < channelCount; channel += 1) sum += channels[channel][frame] ?? 0
    mono[frame] = sum / channelCount
  }
  return mono
}

/**
 * Resample mono PCM to `targetRate` by linear interpolation.
 *
 * ponytail: linear interpolation is a weak anti-alias filter; if enrollment quality complaints
 * appear on 48 kHz devices, replace with a windowed-sinc resampler.
 */
export function resampleLinear(input: Float32Array, inputRate: number, targetRate: number): Float32Array {
  if (inputRate <= 0 || targetRate <= 0 || input.length === 0) return new Float32Array(0)
  if (inputRate === targetRate) return input.slice()
  const ratio = inputRate / targetRate
  const outputLength = Math.max(1, Math.floor(input.length / ratio))
  const output = new Float32Array(outputLength)
  for (let index = 0; index < outputLength; index += 1) {
    const position = index * ratio
    const lower = Math.floor(position)
    const upper = Math.min(lower + 1, input.length - 1)
    const fraction = position - lower
    output[index] = input[lower] + (input[upper] - input[lower]) * fraction
  }
  return output
}

function clampToInt16(sample: number): number {
  const clamped = Math.max(-1, Math.min(1, sample))
  return clamped < 0 ? Math.round(clamped * 0x8000) : Math.round(clamped * 0x7fff)
}

/** Serialize mono float PCM to a PCM16 little-endian WAV container. */
export function encodeWavPcm16(samples: Float32Array, sampleRate: number): Blob {
  const dataBytes = samples.length * 2
  const buffer = new ArrayBuffer(44 + dataBytes)
  const view = new DataView(buffer)
  writeAscii(view, 0, 'RIFF')
  view.setUint32(4, 36 + dataBytes, true)
  writeAscii(view, 8, 'WAVE')
  writeAscii(view, 12, 'fmt ')
  view.setUint32(16, 16, true)
  view.setUint16(20, 1, true)
  view.setUint16(22, TARGET_CHANNELS, true)
  view.setUint32(24, sampleRate, true)
  view.setUint32(28, sampleRate * TARGET_CHANNELS * (BITS_PER_SAMPLE / 8), true)
  view.setUint16(32, TARGET_CHANNELS * (BITS_PER_SAMPLE / 8), true)
  view.setUint16(34, BITS_PER_SAMPLE, true)
  writeAscii(view, 36, 'data')
  view.setUint32(40, dataBytes, true)
  for (let index = 0; index < samples.length; index += 1) {
    view.setInt16(44 + index * 2, clampToInt16(samples[index]), true)
  }
  return new Blob([buffer], { type: 'audio/wav' })
}

function writeAscii(view: DataView, offset: number, text: string) {
  for (let index = 0; index < text.length; index += 1) view.setUint8(offset + index, text.charCodeAt(index))
}

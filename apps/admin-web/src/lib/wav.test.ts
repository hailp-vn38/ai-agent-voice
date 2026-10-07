import { describe, expect, it } from 'vitest'

import { TARGET_SAMPLE_RATE, downmixToMono, encodeWavPcm16, resampleLinear } from './wav'

async function bytesOf(blob: Blob): Promise<DataView> {
  return new DataView(await blob.arrayBuffer())
}

describe('downmixToMono', () => {
  it('averages channels and passes a single channel through', () => {
    const left = new Float32Array([1, 0, -1])
    const right = new Float32Array([0, 1, -1])
    expect(Array.from(downmixToMono([left, right], 2))).toEqual([0.5, 0.5, -1])
    expect(downmixToMono([left], 1)).toBe(left)
  })
})

describe('resampleLinear', () => {
  it('halves the length from 32 kHz to 16 kHz', () => {
    const input = new Float32Array(32)
    const output = resampleLinear(input, 32_000, TARGET_SAMPLE_RATE)
    expect(output.length).toBe(16)
  })

  it('is a no-op at the target rate and preserves a ramp', () => {
    const input = new Float32Array([0, 1, 2, 3])
    expect(resampleLinear(input, TARGET_SAMPLE_RATE, TARGET_SAMPLE_RATE)).toEqual(input)
    const upsampled = resampleLinear(new Float32Array([0, 2]), 8_000, 16_000)
    expect(Array.from(upsampled)).toEqual([0, 1, 2, 2])
  })
})

describe('encodeWavPcm16', () => {
  it('writes a PCM16 mono 16 kHz WAV header and clamped samples', async () => {
    const samples = new Float32Array([0, 1, -1, 2])
    const blob = encodeWavPcm16(samples, TARGET_SAMPLE_RATE)
    expect(blob.type).toBe('audio/wav')
    expect(blob.size).toBe(44 + samples.length * 2)

    const view = await bytesOf(blob)
    const ascii = (offset: number, length: number) =>
      String.fromCharCode(...Array.from({ length }, (_, index) => view.getUint8(offset + index)))
    expect(ascii(0, 4)).toBe('RIFF')
    expect(ascii(8, 4)).toBe('WAVE')
    expect(ascii(36, 4)).toBe('data')
    expect(view.getUint32(24, true)).toBe(TARGET_SAMPLE_RATE)
    expect(view.getUint16(22, true)).toBe(1)
    expect(view.getUint16(34, true)).toBe(16)
    expect(view.getInt16(44, true)).toBe(0)
    expect(view.getInt16(46, true)).toBe(0x7fff)
    expect(view.getInt16(48, true)).toBe(-0x8000)
    expect(view.getInt16(50, true)).toBe(0x7fff)
  })
})

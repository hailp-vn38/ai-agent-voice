/** Deterministic visuals derived from a stable key, so the same record always looks the same. */

function hash(text: string): number {
  let value = 2166136261
  for (const char of text) {
    value ^= char.codePointAt(0) ?? 0
    value = Math.imul(value, 16777619)
  }
  return value >>> 0
}

/** A hue (0-359) for avatar gradients and card accents. */
export function identityHue(key: string): number {
  return hash(key) % 360
}

/** The first visible character, upper-cased, for monogram avatars. */
export function monogram(name: string, fallback: string): string {
  return Array.from(name.trim())[0]?.toLocaleUpperCase() || fallback
}

/** Bar heights (18-100, percent) that look like a short waveform and are unique per key. */
export function voiceprint(key: string, bars = 36): number[] {
  let state = hash(key) || 1
  return Array.from({ length: bars }, (_, index) => {
    state ^= state << 13
    state ^= state >>> 17
    state ^= state << 5
    const noise = ((state >>> 0) % 1000) / 1000
    const envelope = 0.45 + 0.55 * Math.sin((Math.PI * (index + 0.5)) / bars)
    return Math.round(18 + 82 * noise * envelope)
  })
}

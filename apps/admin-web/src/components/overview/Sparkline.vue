<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{
  /** Values in 0–100, oldest first. */
  values: number[]
  label: string
}>()

const WIDTH = 240
const HEIGHT = 48

const points = computed(() => {
  if (props.values.length < 2) return []
  const step = WIDTH / (props.values.length - 1)
  return props.values.map((value, index) => {
    const clamped = Math.min(100, Math.max(0, value))
    return [index * step, HEIGHT - 2 - (clamped / 100) * (HEIGHT - 4)] as const
  })
})
const line = computed(() => points.value.map(([x, y], i) => `${i ? 'L' : 'M'}${x.toFixed(1)},${y.toFixed(1)}`).join(' '))
const area = computed(() => (points.value.length ? `${line.value} L${WIDTH},${HEIGHT} L0,${HEIGHT} Z` : ''))
const last = computed(() => points.value.at(-1))
</script>

<template>
  <svg :viewBox="`0 0 ${WIDTH} ${HEIGHT}`" preserveAspectRatio="none" class="h-12 w-full overflow-visible" role="img" :aria-label="label">
    <defs>
      <linearGradient id="overview-sparkline-fill" x1="0" x2="0" y1="0" y2="1">
        <stop offset="0%" stop-color="var(--studio-cyan)" stop-opacity="0.35" />
        <stop offset="100%" stop-color="var(--studio-cyan)" stop-opacity="0" />
      </linearGradient>
    </defs>
    <template v-if="points.length">
      <path :d="area" fill="url(#overview-sparkline-fill)" />
      <path :d="line" fill="none" stroke="var(--studio-cyan)" stroke-width="2" vector-effect="non-scaling-stroke" stroke-linejoin="round" />
      <circle v-if="last" :cx="last[0]" :cy="last[1]" r="3" fill="var(--studio-cyan)" />
    </template>
    <line v-else x1="0" :y1="HEIGHT - 2" :x2="WIDTH" :y2="HEIGHT - 2" stroke="var(--border)" stroke-dasharray="4 4" />
  </svg>
</template>

<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{
  label: string
  /** 0–100, or null when the metric is unavailable. */
  percentage: number | null
  detail?: string
}>()

const RADIUS = 42
const CIRCUMFERENCE = 2 * Math.PI * RADIUS
// Leave a gap at the bottom so the dial reads as a gauge, not a donut.
const ARC = CIRCUMFERENCE * 0.75

const value = computed(() =>
  props.percentage != null && Number.isFinite(props.percentage)
    ? Math.min(100, Math.max(0, props.percentage))
    : null,
)
const dash = computed(() => `${((value.value ?? 0) / 100) * ARC} ${CIRCUMFERENCE}`)
const tone = computed(() => {
  if (value.value == null) return 'text-muted-foreground'
  if (value.value >= 90) return 'text-danger'
  if (value.value >= 75) return 'text-warning'
  return 'text-studio-cyan'
})
</script>

<template>
  <div class="flex min-w-0 flex-col items-center text-center">
    <div
      class="relative size-28"
      role="meter"
      :aria-label="label"
      :aria-valuenow="value ?? undefined"
      aria-valuemin="0"
      aria-valuemax="100"
    >
      <svg viewBox="0 0 100 100" class="size-full rotate-[135deg]" aria-hidden="true">
        <circle
          cx="50" cy="50" :r="RADIUS" fill="none" stroke="currentColor" stroke-width="8" stroke-linecap="round"
          class="text-muted" :stroke-dasharray="`${ARC} ${CIRCUMFERENCE}`"
        />
        <circle
          cx="50" cy="50" :r="RADIUS" fill="none" stroke="currentColor" stroke-width="8" stroke-linecap="round"
          class="transition-[stroke-dasharray] duration-700 ease-out" :class="tone" :stroke-dasharray="dash"
        />
      </svg>
      <div class="absolute inset-0 flex flex-col items-center justify-center">
        <span class="text-xl font-semibold tabular-nums">{{ value == null ? '—' : `${Math.round(value)}%` }}</span>
        <span class="text-[11px] font-medium uppercase tracking-wider text-muted-foreground">{{ label }}</span>
      </div>
    </div>
    <p v-if="detail" class="mt-1 max-w-full truncate text-xs text-muted-foreground" :title="detail">{{ detail }}</p>
  </div>
</template>

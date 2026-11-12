<script setup lang="ts">
defineProps<{ active?: boolean }>()
</script>

<template>
  <svg viewBox="0 0 40 40" fill="none" aria-hidden="true" focusable="false">
    <path d="M23 6H12a8 8 0 0 0-8 8v8a8 8 0 0 0 8 8h2l6 5v-5h3a9 9 0 0 0 9-9v-3" fill="currentColor" fill-opacity=".08" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
    <g v-if="active" class="mark-wave" fill="currentColor">
      <rect x="11" y="15" width="2.5" height="10" rx="1.25" />
      <rect x="17" y="12" width="2.5" height="16" rx="1.25" />
      <rect x="23" y="16" width="2.5" height="8" rx="1.25" />
    </g>
    <path v-else d="M18 12c1 4 2 6 6 7-4 1-5 3-6 7-1-4-2-6-6-7 4-1 5-3 6-7Z" fill="currentColor" fill-opacity=".16" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" />
    <path d="M31 2c.7 3.1 1.8 4.3 5 5-3.2.7-4.3 1.9-5 5-.7-3.1-1.8-4.3-5-5 3.2-.7 4.3-1.9 5-5Z" fill="currentColor" />
  </svg>
</template>

<style scoped>
.mark-wave rect { transform-box: fill-box; transform-origin: center; animation: mark-voice 1.2s ease-in-out infinite alternate; }
.mark-wave rect:nth-child(2) { animation-delay: -.5s; }
.mark-wave rect:nth-child(3) { animation-delay: -.8s; }
@keyframes mark-voice { from { transform: scaleY(.4); opacity: .55; } to { transform: scaleY(1); opacity: 1; } }
@media (prefers-reduced-motion: reduce) { .mark-wave rect { animation: none; } }
</style>

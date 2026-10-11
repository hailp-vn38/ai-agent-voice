<script setup lang="ts">
import { useI18n } from '@/composables/useI18n'

const { t } = useI18n()
</script>

<template>
  <div class="grid gap-4" role="status" aria-busy="true" :aria-label="t('common.loading')">
    <div v-for="slot in 6" :key="slot" class="studio-panel card-shimmer relative min-h-52 overflow-hidden p-5" aria-hidden="true">
      <div class="size-11 rounded-xl bg-muted/60" />
      <div class="mt-5 h-5 w-2/3 rounded bg-muted/60" />
      <div class="mt-3 h-3 w-1/2 rounded bg-muted/40" />
      <div class="mt-6 h-3 w-full rounded bg-muted/40" />
      <div class="mt-3 h-3 w-3/4 rounded bg-muted/40" />
    </div>
  </div>
</template>

<style scoped>
.card-shimmer::after {
  position: absolute;
  inset: 0;
  content: '';
  background: linear-gradient(90deg, transparent, var(--muted), transparent);
  transform: translateX(-100%);
  animation: shimmer 1.6s infinite;
}

@keyframes shimmer {
  to { transform: translateX(100%); }
}

@media (prefers-reduced-motion: reduce) {
  .card-shimmer::after { animation: none; }
}
</style>

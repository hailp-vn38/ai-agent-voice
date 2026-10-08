<script setup lang="ts">
import type { Component } from 'vue'

defineProps<{
  label: string
  value: string | number
  hint: string
  icon: Component
}>()
</script>

<template>
  <div class="studio-panel flex min-h-36 flex-col justify-between p-5">
    <div class="flex items-center justify-between gap-3">
      <p class="text-sm font-medium text-muted-foreground">{{ label }}</p>
      <div class="flex size-10 items-center justify-center rounded-xl bg-primary/10 text-studio-violet">
        <component :is="icon" class="size-5" aria-hidden="true" />
      </div>
    </div>
    <div>
      <p class="text-3xl font-semibold tracking-tight tabular-nums">{{ value }}</p>
      <p class="mt-1 text-xs text-muted-foreground">{{ hint }}</p>
    </div>
  </div>
</template>

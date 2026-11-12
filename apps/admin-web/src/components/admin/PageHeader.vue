<script setup lang="ts">
import HeroScene, { type HeroTheme } from './HeroScene.vue'

defineProps<{
  eyebrow?: string
  theme?: HeroTheme
  title: string
  description?: string
}>()
</script>

<template>
  <header :class="{ 'hero-surface': theme }">
    <div class="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
      <div class="min-w-0">
        <p v-if="eyebrow" class="text-sm font-medium text-primary">{{ eyebrow }}</p>
        <h1 class="mt-1 text-3xl font-semibold tracking-tight">{{ title }}</h1>
        <p v-if="description" class="mt-2 max-w-3xl text-sm leading-relaxed text-muted-foreground">{{ description }}</p>
      </div>
      <div v-if="$slots.actions" class="flex shrink-0 flex-wrap items-center gap-2">
        <slot name="actions" />
      </div>
    </div>
    <HeroScene v-if="theme" :theme="theme" surface />
  </header>
</template>

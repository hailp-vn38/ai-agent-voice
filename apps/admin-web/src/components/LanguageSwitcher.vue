<script setup lang="ts">
import { Languages } from '@lucide/vue'

import { useI18n, type Locale } from '@/composables/useI18n'

const { locale, locales, t, setLocale } = useI18n()

function onChange(event: Event) {
  const value = (event.target as HTMLSelectElement).value
  if (value === 'en' || value === 'vi') setLocale(value as Locale)
}
</script>

<template>
  <div class="flex items-center justify-between gap-2">
    <span class="flex items-center gap-1.5 text-muted-foreground">
      <Languages class="size-3.5" aria-hidden="true" />
      {{ t('app.language') }}
    </span>
    <select
      class="h-7 rounded-md border border-input bg-background px-2 text-xs outline-none focus:border-ring focus:ring-2 focus:ring-ring/30"
      :value="locale"
      :aria-label="t('app.language')"
      @change="onChange"
    >
      <option v-for="option in locales" :key="option.value" :value="option.value">
        {{ option.label }}
      </option>
    </select>
  </div>
</template>

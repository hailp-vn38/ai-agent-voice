<script setup lang="ts">
import { AudioWaveform } from '@lucide/vue'

import LanguageSwitcher from '@/components/LanguageSwitcher.vue'
import ThemeToggle from '@/components/ThemeToggle.vue'
import { useI18n } from '@/composables/useI18n'
import { navigation } from '@/config/navigation'

defineEmits<{ navigate: [] }>()

const { t } = useI18n()
</script>

<template>
  <div class="flex h-full min-h-0 flex-col bg-sidebar text-sidebar-foreground">
    <div class="flex h-16 shrink-0 items-center gap-3 border-b px-5">
      <div class="flex size-9 items-center justify-center rounded-lg bg-primary text-primary-foreground">
        <AudioWaveform class="size-5" />
      </div>
      <div class="min-w-0">
        <p class="truncate text-sm font-semibold">{{ t('app.brand') }}</p>
        <p class="truncate text-xs text-muted-foreground">{{ t('app.subtitle') }}</p>
      </div>
    </div>
    <nav class="flex-1 space-y-1 overflow-y-auto p-3">
      <RouterLink
        v-for="item in navigation"
        :key="item.to"
        :to="item.to"
        class="flex items-center gap-3 rounded-md px-3 py-2 text-sm text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
        active-class="bg-sidebar-accent text-sidebar-accent-foreground"
        @click="$emit('navigate')"
      >
        <component :is="item.icon" class="size-4" />
        <span>{{ t(item.label) }}</span>
      </RouterLink>
    </nav>
    <div class="shrink-0 border-t p-4 text-xs text-muted-foreground">
      <div class="mb-3 flex items-center justify-between gap-2">
        <span class="text-sidebar-foreground">{{ t('app.theme') }}</span>
        <ThemeToggle />
      </div>
      <LanguageSwitcher class="mb-3" />
      <p class="font-medium text-foreground">{{ t('app.surfaceTitle') }}</p>
      <p class="mt-1 leading-relaxed">{{ t('app.surfaceDescription') }}</p>
    </div>
  </div>
</template>

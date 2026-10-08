<script setup lang="ts">
import { AudioWaveform } from '@lucide/vue'

import LanguageSwitcher from '@/components/LanguageSwitcher.vue'
import ThemeToggle from '@/components/ThemeToggle.vue'
import { useI18n } from '@/composables/useI18n'
import { navigationGroups } from '@/config/navigation'

defineEmits<{ navigate: [] }>()
const { t } = useI18n()
</script>

<template>
  <div class="flex h-full min-h-0 flex-col bg-sidebar text-sidebar-foreground">
    <div class="flex h-20 shrink-0 items-center gap-3 border-b border-border/60 px-5">
      <div class="studio-logo flex size-10 items-center justify-center rounded-xl">
        <AudioWaveform class="size-5" aria-hidden="true" />
      </div>
      <div class="min-w-0">
        <p class="truncate text-sm font-bold tracking-tight">{{ t('app.brand') }}</p>
        <p class="truncate text-xs text-muted-foreground">{{ t('app.subtitle') }}</p>
      </div>
    </div>

    <nav class="flex-1 space-y-6 overflow-y-auto px-3 py-6" :aria-label="t('nav.open')">
      <section v-for="group in navigationGroups" :key="group.label" class="space-y-1">
        <h2 class="px-3 pb-2 text-[10px] font-semibold uppercase tracking-[0.16em] text-muted-foreground">
          {{ t(group.label) }}
        </h2>
        <RouterLink
          v-for="item in group.items"
          :key="item.to"
          :to="item.to"
          class="flex items-center gap-3 rounded-lg border border-transparent px-3 py-2.5 text-sm text-sidebar-foreground/75 transition-colors hover:bg-sidebar-accent/65 hover:text-sidebar-accent-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          active-class="!border-studio-violet/35 !bg-studio-violet/15 !font-semibold !text-studio-violet"
          @click="$emit('navigate')"
        >
          <component :is="item.icon" class="size-[18px] shrink-0" aria-hidden="true" />
          <span>{{ t(item.label) }}</span>
        </RouterLink>
      </section>
    </nav>

    <div class="shrink-0 border-t border-border/60 p-4">
      <div class="flex items-center justify-between gap-2 text-xs">
        <span class="font-medium">{{ t('app.theme') }}</span>
        <ThemeToggle />
      </div>
      <LanguageSwitcher class="mt-2" />
      <p class="mt-3 text-xs text-muted-foreground">{{ t('studio.sidebarHint') }}</p>
    </div>
  </div>
</template>

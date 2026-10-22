<script setup lang="ts">
import { AudioWaveform, PanelLeftClose, PanelLeftOpen } from '@lucide/vue'

import LanguageSwitcher from '@/components/LanguageSwitcher.vue'
import ThemeToggle from '@/components/ThemeToggle.vue'
import { useI18n } from '@/composables/useI18n'
import { navigationGroups } from '@/config/navigation'

withDefaults(defineProps<{ collapsible?: boolean; collapsed?: boolean }>(), {
  collapsible: false,
  collapsed: false,
})
defineEmits<{ navigate: []; toggle: [] }>()
const { t } = useI18n()
</script>

<template>
  <div class="flex h-full w-full min-w-0 flex-col bg-sidebar text-sidebar-foreground">
    <div class="flex h-20 shrink-0 items-center border-b border-border/60 transition-[padding] duration-300 motion-reduce:transition-none"
      :class="collapsed ? 'justify-center px-2' : 'gap-3 px-5'">
      <template v-if="!collapsed">
        <div class="studio-logo flex size-10 shrink-0 items-center justify-center rounded-xl">
          <AudioWaveform class="size-5" aria-hidden="true" />
        </div>
        <div class="min-w-0 flex-1">
          <p class="truncate text-sm font-bold tracking-tight">{{ t('app.brand') }}</p>
          <p class="truncate text-xs text-muted-foreground">{{ t('app.subtitle') }}</p>
        </div>
      </template>
      <button v-if="collapsible" type="button"
        class="flex size-9 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
        :aria-label="collapsed ? 'Expand sidebar' : 'Collapse sidebar'"
        :title="collapsed ? 'Expand sidebar' : 'Collapse sidebar'"
        :aria-expanded="!collapsed"
        @click="$emit('toggle')">
        <PanelLeftOpen v-if="collapsed" class="size-4" />
        <PanelLeftClose v-else class="size-4" />
      </button>
    </div>

    <nav class="flex-1 space-y-5 overflow-x-hidden overflow-y-auto py-5 transition-[padding] duration-300 motion-reduce:transition-none"
      :class="collapsed ? 'px-2' : 'px-3'" :aria-label="t('nav.open')">
      <section v-for="(group, groupIndex) in navigationGroups" :key="group.label"
        class="space-y-1" :class="collapsed && groupIndex > 0 ? 'border-t border-border/60 pt-4' : ''">
        <h2 v-if="!collapsed" class="px-3 pb-2 text-[10px] font-semibold uppercase tracking-[0.16em] text-muted-foreground">
          {{ t(group.label) }}
        </h2>
        <RouterLink v-for="item in group.items" :key="item.to" :to="item.to"
          class="flex h-10 items-center rounded-lg border border-transparent text-sm text-sidebar-foreground/75 transition-[background-color,color,padding] duration-200 hover:bg-sidebar-accent/65 hover:text-sidebar-accent-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          :class="collapsed ? 'justify-center px-0' : 'gap-3 px-3'"
          :aria-label="t(item.label)"
          :title="collapsed ? t(item.label) : undefined"
          active-class="!border-studio-violet/35 !bg-studio-violet/15 !font-semibold !text-studio-violet"
          @click="$emit('navigate')">
          <component :is="item.icon" class="size-[18px] shrink-0" aria-hidden="true" />
          <span v-if="!collapsed" class="min-w-0 truncate">{{ t(item.label) }}</span>
        </RouterLink>
      </section>
    </nav>

    <div class="shrink-0 border-t border-border/60 transition-[padding] duration-300 motion-reduce:transition-none"
      :class="collapsed ? 'flex justify-center p-2' : 'p-4'">
      <ThemeToggle v-if="collapsed" />
      <template v-else>
        <div class="flex items-center justify-between gap-2 text-xs">
          <span class="font-medium">{{ t('app.theme') }}</span>
          <ThemeToggle />
        </div>
        <LanguageSwitcher class="mt-2" />
        <p class="mt-3 text-xs text-muted-foreground">{{ t('studio.sidebarHint') }}</p>
      </template>
    </div>
  </div>
</template>

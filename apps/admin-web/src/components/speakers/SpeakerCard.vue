<script setup lang="ts">
import { AudioLines, CalendarClock, CheckCircle2, Pencil, Trash2 } from '@lucide/vue'

import type { SpeakerSummary } from '@/api/types/speakers'
import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { useI18n } from '@/composables/useI18n'

defineProps<{ speaker: SpeakerSummary }>()
const emit = defineEmits<{ edit: [key: string]; delete: [speaker: SpeakerSummary] }>()
const { t, formatDateTime } = useI18n()
</script>

<template>
  <article
    class="studio-panel group relative flex min-w-0 flex-col justify-between gap-4 p-5 transition-all duration-200 hover:-translate-y-0.5 hover:border-studio-violet/40 hover:shadow-lg focus-within:border-studio-violet/50"
    data-speaker-card
  >
    <!-- A full-card link is a sibling of the actions, never a parent of buttons. -->
    <RouterLink
      :to="{ name: 'speaker-detail', params: { speakerKey: speaker.key } }"
      :aria-label="t('speakers.cardOpen', { name: speaker.name })"
      class="absolute inset-0 z-10 cursor-pointer rounded-2xl focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      data-speaker-open
    />
    <div class="pointer-events-none flex min-w-0 items-start gap-3">
      <span class="flex size-12 shrink-0 items-center justify-center rounded-xl bg-studio-violet/10 text-studio-violet">
        <AudioLines class="size-6" aria-hidden="true" />
      </span>
      <div class="min-w-0 flex-1">
        <h2 class="truncate text-lg font-semibold">{{ speaker.name }}</h2>
        <p class="mt-1 line-clamp-2 min-h-10 break-words text-sm leading-5 text-muted-foreground">
          {{ speaker.description || t('speakers.noDescription') }}
        </p>
      </div>
      <div class="pointer-events-auto relative z-20 shrink-0" data-speaker-menu>
        <ActionMenu :label="t('speakers.actionsLabel', { name: speaker.name })">
          <MenuItem data-speaker-edit @select="emit('edit', speaker.key)">
            <Pencil class="size-4 shrink-0" aria-hidden="true" />
            {{ t('common.edit') }}
          </MenuItem>
          <div class="my-1 h-px bg-border" role="separator" />
          <MenuItem variant="danger" data-speaker-delete @select="emit('delete', speaker)">
            <Trash2 class="size-4 shrink-0" aria-hidden="true" />
            {{ t('common.delete') }}
          </MenuItem>
        </ActionMenu>
      </div>
    </div>

    <div class="pointer-events-none flex min-w-0 flex-wrap items-center justify-between gap-3 border-t border-border/70 pt-4 text-xs">
      <span class="inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 font-medium"
        :class="speaker.enabled ? 'bg-success/10 text-success-foreground' : 'bg-muted text-muted-foreground'">
        <CheckCircle2 v-if="speaker.enabled" class="size-3.5" aria-hidden="true" />
        <span v-else class="size-1.5 rounded-full bg-current" aria-hidden="true" />
        {{ speaker.enabled ? t('speakers.enabled') : t('speakers.disabled') }}
      </span>
      <span class="inline-flex items-center min-w-0 gap-1.5 text-muted-foreground">
        <CalendarClock class="size-3.5 shrink-0" aria-hidden="true" />
        {{ formatDateTime(new Date(speaker.updated_at * 1000)) }}
      </span>
    </div>
  </article>
</template>

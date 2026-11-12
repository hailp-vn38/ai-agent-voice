<script setup lang="ts">
import { CalendarClock, Pencil, Trash2 } from '@lucide/vue'
import { computed } from 'vue'

import type { SpeakerSummary } from '@/api/types/speakers'
import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { useI18n } from '@/composables/useI18n'
import { identityHue, monogram, voiceprint } from '@/lib/visualIdentity'

const props = defineProps<{ speaker: SpeakerSummary }>()
/** A decorative, per-speaker waveform. It is identity colour, not real audio data. */
const bars = computed(() => voiceprint(props.speaker.key))
const emit = defineEmits<{ edit: [key: string]; delete: [speaker: SpeakerSummary] }>()
const { t, formatDateTime } = useI18n()
</script>

<template>
  <article class="dir-card speaker-card group" :data-enabled="speaker.enabled" :style="{ '--hue': identityHue(speaker.key) }" data-speaker-card>
    <RouterLink :to="{ name: 'speaker-detail', params: { speakerKey: speaker.key } }" :aria-label="t('speakers.cardOpen', { name: speaker.name })" class="dir-card-hit" data-speaker-open />
    <div class="dir-card-head">
      <span class="dir-avatar" aria-hidden="true">{{ monogram(speaker.name, 'S') }}</span>
      <div class="dir-card-title">
        <h2>{{ speaker.name }}</h2>
        <span class="dir-pill" :data-enabled="speaker.enabled"><i aria-hidden="true" />{{ speaker.enabled ? t('speakers.enabled') : t('speakers.disabled') }}</span>
      </div>
      <div class="dir-card-raise" data-speaker-menu>
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
    <p class="dir-card-desc">{{ speaker.description || t('speakers.noDescription') }}</p>
    <div class="speaker-voiceprint" aria-hidden="true"><i v-for="(height, index) in bars" :key="index" :style="{ height: `${height}%` }" /></div>
    <div class="dir-card-foot">
      <span><CalendarClock :size="14" aria-hidden="true" />{{ formatDateTime(new Date(speaker.updated_at * 1000)) }}</span>
      <span class="dir-card-push dir-mono">{{ t('speakers.revision') }} {{ speaker.revision }}</span>
    </div>
  </article>
</template>

<style scoped src="../admin/studio-directory-cards.css"></style>

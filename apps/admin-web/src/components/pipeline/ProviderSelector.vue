<script setup lang="ts">
import { ChevronDown, Plus } from '@lucide/vue'
import { computed } from 'vue'

import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { useI18n } from '@/composables/useI18n'
import type { ProviderInstance, ProviderType } from '@/domain/admin'

const props = defineProps<{
  type: ProviderType
  candidates: ProviderInstance[]
}>()

const emit = defineEmits<{ select: [providerId: string] }>()

const { t, providerTypeLabel, providerStatusLabel } = useI18n()

const label = computed(() => t('pipeline.selectProvider', { type: providerTypeLabel(props.type) }))
const empty = computed(() => props.candidates.length === 0)
</script>

<template>
  <div>
    <ActionMenu :label="label" variant="outline" size="sm" align="start" panel-width="16rem" :disabled="empty">
      <template #trigger>
        <Plus class="size-3.5" aria-hidden="true" />
        {{ t('pipeline.selectCta') }}
        <ChevronDown class="size-3.5" aria-hidden="true" />
      </template>

      <p class="px-3 pb-1.5 pt-2 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        {{ t('pipeline.instances', { type: providerTypeLabel(type) }) }}
      </p>
      <MenuItem
        v-for="provider in candidates"
        :key="provider.id"
        @select="emit('select', provider.id)"
      >
        <span class="flex min-w-0 flex-1 flex-col items-start gap-0.5">
          <span class="w-full truncate text-sm font-medium">{{ provider.name }}</span>
          <span class="w-full truncate font-mono text-xs text-muted-foreground">{{ provider.adapter }}</span>
        </span>
        <span class="shrink-0 text-[11px] text-muted-foreground">{{ providerStatusLabel(provider.status) }}</span>
      </MenuItem>
    </ActionMenu>

    <p v-if="empty" class="text-xs text-muted-foreground">
      {{ t('pipeline.noCandidates', { type: providerTypeLabel(type) }) }}
    </p>
  </div>
</template>

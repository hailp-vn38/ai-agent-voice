<script setup lang="ts">
import { Eye, Link2Off, Pencil, TriangleAlert } from '@lucide/vue'
import { computed } from 'vue'

import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { useI18n } from '@/composables/useI18n'
import type { ProviderInstance, ProviderType } from '@/domain/admin'

const props = defineProps<{
  type: ProviderType
  provider: ProviderInstance
  /** Names of the other templates that also bind this provider instance. */
  sharedWith?: string[]
}>()

const emit = defineEmits<{
  view: [provider: ProviderInstance]
  edit: [provider: ProviderInstance]
  unlink: [type: ProviderType]
}>()

const { t, providerTypeLabel } = useI18n()

const shared = computed(() => (props.sharedWith?.length ?? 0) > 0)
</script>

<template>
  <ActionMenu :label="t('pipeline.actions', { type: providerTypeLabel(type) })" panel-width="16rem">
    <MenuItem @select="emit('view', provider)">
      <Eye class="size-4 shrink-0" aria-hidden="true" />
      {{ t('pipeline.viewProvider') }}
    </MenuItem>

    <MenuItem @select="emit('edit', provider)">
      <Pencil class="size-4 shrink-0" aria-hidden="true" />
      {{ t('pipeline.editProvider') }}
    </MenuItem>

    <div v-if="shared" class="mx-1 my-1 flex gap-2 rounded-md bg-muted/60 px-2.5 py-2">
      <TriangleAlert class="mt-0.5 size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
      <p class="text-[11px] leading-relaxed text-muted-foreground">
        {{ t('pipeline.sharedWarning', { count: t('count.otherTemplates', { count: sharedWith?.length ?? 0 }) }) }}
      </p>
    </div>

    <div class="my-1 h-px bg-border" role="separator" />

    <MenuItem variant="danger" @select="emit('unlink', type)">
      <Link2Off class="size-4 shrink-0" aria-hidden="true" />
      {{ t('pipeline.unlink') }}
    </MenuItem>
  </ActionMenu>
</template>

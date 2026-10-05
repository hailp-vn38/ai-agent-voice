<script setup lang="ts">
import { Copy, Eye, Link2, Trash2 } from '@lucide/vue'

import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { useI18n } from '@/composables/useI18n'
import type { AgentTemplate } from '@/domain/admin'

defineProps<{ template: AgentTemplate; showViewDetails?: boolean }>()

const { t } = useI18n()

const emit = defineEmits<{
  viewDetails: []
  linkToAgent: []
  copy: []
  delete: []
}>()
</script>

<template>
  <ActionMenu :label="t('templatesActions.label', { name: template.name })" panel-width="14rem">
    <MenuItem v-if="showViewDetails !== false" @select="emit('viewDetails')">
      <Eye class="size-4 shrink-0" aria-hidden="true" />
      {{ t('templatesActions.viewDetails') }}
    </MenuItem>
    <MenuItem @select="emit('linkToAgent')">
      <Link2 class="size-4 shrink-0" aria-hidden="true" />
      {{ t('templatesActions.linkToAgent') }}
    </MenuItem>
    <MenuItem @select="emit('copy')">
      <Copy class="size-4 shrink-0" aria-hidden="true" />
      {{ t('templatesActions.copy') }}
    </MenuItem>
    <div class="my-1 h-px bg-border" role="separator" />
    <MenuItem variant="danger" @select="emit('delete')">
      <Trash2 class="size-4 shrink-0" aria-hidden="true" />
      {{ t('templatesActions.delete') }}
    </MenuItem>
  </ActionMenu>
</template>

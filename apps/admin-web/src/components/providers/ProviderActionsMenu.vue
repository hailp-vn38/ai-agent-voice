<script setup lang="ts">
import { Copy, Eye, Link2, Pencil, Trash2 } from '@lucide/vue'

import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { useI18n } from '@/composables/useI18n'
import type { ProviderInstance } from '@/domain/admin'

defineProps<{
  provider: ProviderInstance
}>()

const emit = defineEmits<{
  view: []
  edit: []
  link: []
  duplicate: []
  delete: []
}>()

const { t } = useI18n()
</script>

<template>
  <ActionMenu :label="t('providers.actionsLabel', { name: provider.name })">
    <MenuItem @select="emit('view')">
      <Eye class="size-4 shrink-0" aria-hidden="true" />
      {{ t('pipeline.viewProvider') }}
    </MenuItem>

    <MenuItem @select="emit('edit')">
      <Pencil class="size-4 shrink-0" aria-hidden="true" />
      {{ t('common.edit') }}
    </MenuItem>

    <MenuItem @select="emit('link')">
      <Link2 class="size-4 shrink-0" aria-hidden="true" />
      {{ t('providers.linkTemplate') }}
    </MenuItem>

    <MenuItem @select="emit('duplicate')">
      <Copy class="size-4 shrink-0" aria-hidden="true" />
      {{ t('common.duplicate') }}
    </MenuItem>

    <div class="my-1 h-px bg-border" role="separator" />

    <MenuItem variant="danger" @select="emit('delete')">
      <Trash2 class="size-4 shrink-0" aria-hidden="true" />
      {{ t('providers.delete') }}
    </MenuItem>
  </ActionMenu>
</template>
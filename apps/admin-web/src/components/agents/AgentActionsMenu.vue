<script setup lang="ts">
import { Trash2 } from '@lucide/vue'

import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { useI18n } from '@/composables/useI18n'

const emit = defineEmits<{ deleteAgent: [] }>()

const { t } = useI18n()
</script>

<template>
  <ActionMenu :label="t('common.actions')" panel-width="12rem">
    <MenuItem variant="danger" @select="emit('deleteAgent')">
      <Trash2 class="size-4 shrink-0" aria-hidden="true" />
      {{ t('agents.delete') }}
    </MenuItem>
  </ActionMenu>
</template>

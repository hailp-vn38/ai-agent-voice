<script setup lang="ts">
import { TriangleAlert } from '@lucide/vue'

import BaseModal from '@/components/admin/BaseModal.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const { t } = useI18n()

const open = defineModel<boolean>({ required: true })

withDefaults(
  defineProps<{
    title: string
    description?: string
    confirmLabel?: string
    cancelLabel?: string
    tone?: 'default' | 'danger'
  }>(),
  {
    description: '',
    confirmLabel: '',
    cancelLabel: '',
    tone: 'default',
  },
)

const emit = defineEmits<{ confirm: [] }>()

function confirm() {
  emit('confirm')
  open.value = false
}
</script>

<template>
  <BaseModal v-model="open" :title="title" :description="description" width-class="max-w-md">
    <div class="flex gap-3">
      <span
        class="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-full"
        :class="tone === 'danger' ? 'bg-danger/15 text-danger' : 'bg-muted text-muted-foreground'"
        aria-hidden="true"
      >
        <TriangleAlert class="size-4" />
      </span>
      <div class="min-w-0 flex-1 space-y-4 text-sm leading-relaxed text-muted-foreground">
        <slot />
      </div>
    </div>

    <template #footer>
      <div class="flex justify-end gap-2">
        <Button variant="outline" @click="open = false">{{ cancelLabel || t('common.cancel') }}</Button>
        <Button
          :class="tone === 'danger' ? 'bg-danger text-white hover:bg-danger/90' : undefined"
          @click="confirm"
        >
          {{ confirmLabel || t('common.confirm') }}
        </Button>
      </div>
    </template>
  </BaseModal>
</template>

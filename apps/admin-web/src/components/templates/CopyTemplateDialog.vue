<script setup lang="ts">
import { Copy } from '@lucide/vue'
import { ref, watch } from 'vue'

import BaseModal from '@/components/admin/BaseModal.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { AgentTemplate } from '@/domain/admin'

const props = defineProps<{
  /** Omit while the dialog is closed without a target. */
  template?: AgentTemplate
}>()

const open = defineModel<boolean>({ required: true })
const emit = defineEmits<{ copy: [name: string] }>()

const { t } = useI18n()

const name = ref('')

watch(
  () => [open.value, props.template?.id] as const,
  () => {
    if (!open.value || !props.template) return
    name.value = `${props.template.name} Copy`
  },
  { immediate: true },
)

function submit() {
  if (!props.template || !name.value.trim()) return
  emit('copy', name.value.trim())
  open.value = false
}
</script>

<template>
  <BaseModal
    v-model="open"
    :title="t('templateCopy.title', { name: template?.name ?? '' })"
    :description="t('templateCopy.description')"
    width-class="max-w-md"
  >
    <form class="space-y-4" @submit.prevent="submit">
      <label class="block space-y-1.5">
        <span class="text-sm font-medium">{{ t('templateCopy.name') }}</span>
        <input v-model="name" class="admin-input" :placeholder="t('templateForm.namePlaceholder')" required />
      </label>

      <p class="rounded-md bg-muted/60 px-3 py-2.5 text-xs leading-relaxed text-muted-foreground">
        {{ t('templateCopy.hint') }}
      </p>

      <div class="flex justify-end gap-2 pt-1">
        <Button type="button" variant="outline" @click="open = false">{{ t('common.cancel') }}</Button>
        <Button type="submit" :disabled="!name.trim()">
          <Copy class="size-4" />
          {{ t('templateCopy.submit') }}
        </Button>
      </div>
    </form>
  </BaseModal>
</template>

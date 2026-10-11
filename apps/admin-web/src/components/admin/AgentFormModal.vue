<script setup lang="ts">
import { reactive, watch } from 'vue'

import BaseModal from '@/components/admin/BaseModal.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { Agent, AgentTemplate } from '@/domain/admin'

const props = defineProps<{
  agent?: Agent
  templates?: AgentTemplate[]
}>()

const open = defineModel<boolean>({ required: true })

const emit = defineEmits<{
  save: [payload: { name: string; description: string }]
  createWithTemplate: [payload: { agent: { name: string; description: string }; templateId: string }]
}>()

const { t } = useI18n()

const form = reactive({
  name: '',
  description: '',
  templateId: '',
})

watch(
  () => [open.value, props.agent] as const,
  () => {
    if (!open.value) return
    form.name = props.agent?.name ?? ''
    form.description = props.agent?.description ?? ''
    form.templateId = ''
  },
  { immediate: true },
)

function submit() {
  if (!form.name.trim()) return
  if (props.agent) {
    emit('save', { name: form.name.trim(), description: form.description.trim() })
  } else {
    if (!props.templates?.some((template) => template.id === form.templateId)) return
    emit('createWithTemplate', {
      agent: { name: form.name.trim(), description: form.description.trim() },
      templateId: form.templateId,
    })
  }
  open.value = false
}
</script>

<template>
  <BaseModal
    v-model="open"
    :title="agent ? t('agents.edit') : t('agents.createTitle')"
    :description="agent ? t('agents.editDescription') : t('agents.createDescription')"
  >
    <form class="space-y-5" @submit.prevent="submit">
      <fieldset class="space-y-4">
        <legend class="text-sm font-semibold">{{ t('agents.legend') }}</legend>

        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('agents.name') }}</span>
          <input
            v-model="form.name"
            class="admin-input"
            :placeholder="t('agents.namePlaceholder')"
            required
          />
        </label>

        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('agents.descriptionField') }}</span>
          <textarea
            v-model="form.description"
            class="admin-textarea min-h-20"
            :placeholder="t('agents.descriptionPlaceholder')"
          />
        </label>
      </fieldset>

      <fieldset v-if="!agent" class="space-y-4 border-t pt-5">
        <legend class="text-sm font-semibold">{{ t('agents.firstTemplate') }}</legend>
        <p class="text-xs text-muted-foreground">{{ t('agents.firstTemplateHint') }}</p>

        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('agents.firstTemplate') }}</span>
          <select v-model="form.templateId" class="admin-input" required>
            <option disabled value="">{{ t('agents.selectTemplate') }}</option>
            <option v-for="template in templates" :key="template.id" :value="template.id">
              {{ template.name }} · {{ template.language }}
            </option>
          </select>
        </label>
        <p v-if="!templates?.length" class="text-xs text-muted-foreground">
          {{ t('agents.noTemplatesHint') }}
        </p>
      </fieldset>

      <div class="flex justify-end gap-2 pt-1">
        <Button type="button" variant="outline" @click="open = false">{{ t('common.cancel') }}</Button>
        <Button type="submit" :disabled="!agent && !templates?.some((template) => template.id === form.templateId)">
          {{ agent ? t('common.saveChanges') : t('agents.createSubmit') }}
        </Button>
      </div>
    </form>
  </BaseModal>
</template>

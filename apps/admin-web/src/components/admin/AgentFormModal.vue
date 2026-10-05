<script setup lang="ts">
import { computed, reactive, watch } from 'vue'

import BaseModal from '@/components/admin/BaseModal.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import {
  providerTypes,
  templateLanguageOptions,
  type Agent,
  type ProviderInstance,
  type ProviderType,
} from '@/domain/admin'
import type { AgentTemplateInput } from '@/stores/admin'

const props = defineProps<{
  agent?: Agent
  providers?: ProviderInstance[]
}>()

const open = defineModel<boolean>({ required: true })

const emit = defineEmits<{
  save: [payload: { name: string; description: string }]
  createWithTemplate: [payload: { agent: { name: string; description: string }; template: AgentTemplateInput }]
}>()

const { t, providerTypeLabel } = useI18n()

const form = reactive({
  name: '',
  description: '',
  templateName: '',
  language: templateLanguageOptions[0] as string,
  prompt: '',
  providerBindings: {} as Partial<Record<ProviderType, string>>,
})

const editing = computed(() => Boolean(props.agent))

function candidatesFor(type: ProviderType) {
  return (props.providers ?? []).filter((provider) => provider.type === type)
}

watch(
  () => [open.value, props.agent] as const,
  () => {
    if (!open.value) return
    form.name = props.agent?.name ?? ''
    form.description = props.agent?.description ?? ''
    form.templateName = ''
    form.language = templateLanguageOptions[0]
    form.prompt = ''
    form.providerBindings = {}
  },
  { immediate: true },
)

function buildTemplate(): AgentTemplateInput {
  const providerBindings = Object.fromEntries(
    Object.entries(form.providerBindings).filter(([, providerId]) => Boolean(providerId)),
  ) as Partial<Record<ProviderType, string>>
  return {
    name: form.templateName.trim() || `${form.name.trim()} template`,
    language: form.language.trim() || templateLanguageOptions[0],
    prompt: form.prompt,
    providerBindings,
  }
}

function submit() {
  if (!form.name.trim()) return
  if (editing.value) {
    emit('save', { name: form.name.trim(), description: form.description.trim() })
  } else {
    emit('createWithTemplate', {
      agent: { name: form.name.trim(), description: form.description.trim() },
      template: buildTemplate(),
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
          <span class="text-sm font-medium">{{ t('agents.templateName') }}</span>
          <input
            v-model="form.templateName"
            class="admin-input"
            :placeholder="t('agents.templateNamePlaceholder')"
          />
        </label>

        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('templateForm.language') }}</span>
          <input
            v-model="form.language"
            class="admin-input"
            list="agent-template-language-options"
            :placeholder="t('templateForm.languagePlaceholder')"
          />
          <datalist id="agent-template-language-options">
            <option v-for="option in templateLanguageOptions" :key="option" :value="option" />
          </datalist>
        </label>

        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('templateForm.prompt') }}</span>
          <textarea
            v-model="form.prompt"
            class="admin-textarea min-h-32 font-mono text-xs"
            spellcheck="false"
            :placeholder="t('templateForm.promptPlaceholder')"
          />
        </label>

        <div class="space-y-3">
          <p class="text-sm font-medium">{{ t('templateForm.providers') }}</p>
          <p class="text-xs text-muted-foreground">{{ t('templateForm.providersHint') }}</p>
          <div class="grid gap-3 sm:grid-cols-2">
            <label v-for="type in providerTypes" :key="type" class="block space-y-1.5">
              <span class="text-xs font-medium text-muted-foreground">
                {{ providerTypeLabel(type) }}
              </span>
              <select
                v-model="form.providerBindings[type]"
                class="admin-input"
                :aria-label="providerTypeLabel(type)"
              >
                <option value="">{{ t('templateForm.noProvider') }}</option>
                <option v-for="provider in candidatesFor(type)" :key="provider.id" :value="provider.id">
                  {{ provider.name }} · {{ provider.adapter }}
                </option>
              </select>
            </label>
          </div>
        </div>
      </fieldset>

      <div class="flex justify-end gap-2 pt-1">
        <Button type="button" variant="outline" @click="open = false">{{ t('common.cancel') }}</Button>
        <Button type="submit">
          {{ agent ? t('common.saveChanges') : t('agents.createSubmit') }}
        </Button>
      </div>
    </form>
  </BaseModal>
</template>

<script setup lang="ts">
import { Link2 } from '@lucide/vue'
import { computed, ref, watch } from 'vue'

import BaseModal from '@/components/admin/BaseModal.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { Agent, AgentTemplate } from '@/domain/admin'

const props = defineProps<{
  agent: Agent
  /** Global templates the agent does not link yet. */
  availableTemplates: AgentTemplate[]
}>()

const open = defineModel<boolean>({ required: true })
const emit = defineEmits<{ link: [templateId: string, setAsDefault: boolean] }>()

const { t } = useI18n()

const templateId = ref('')
const setAsDefault = ref(false)

watch(
  () => open.value,
  (value) => {
    if (!value) return
    templateId.value = props.availableTemplates[0]?.id ?? ''
    setAsDefault.value = false
  },
  { immediate: true },
)

const selectedTemplate = computed(() =>
  props.availableTemplates.find((template) => template.id === templateId.value),
)
const requiresDefault = computed(() => !props.agent.defaultTemplateId)

function link() {
  if (!templateId.value) return
  emit('link', templateId.value, requiresDefault.value || setAsDefault.value)
  open.value = false
}
</script>

<template>
  <BaseModal
    v-model="open"
    :title="t('linkExisting.title', { agent: agent.name })"
    :description="t('linkExisting.description')"
    width-class="max-w-md"
  >
    <form class="space-y-4" @submit.prevent="link">
      <label class="block space-y-1.5">
        <span class="text-sm font-medium">{{ t('linkExisting.template') }}</span>
        <select v-model="templateId" class="admin-input" required :disabled="availableTemplates.length === 0">
          <option v-for="template in availableTemplates" :key="template.id" :value="template.id">
            {{ template.name }} · {{ template.language || '—' }}
          </option>
        </select>
      </label>

      <label class="flex items-start gap-2.5 rounded-lg border border-border/70 px-3 py-2.5" :class="{ 'cursor-pointer': !requiresDefault }">
        <input
          :checked="requiresDefault || setAsDefault"
          type="checkbox"
          class="mt-1 accent-foreground"
          :disabled="requiresDefault"
          @change="setAsDefault = ($event.target as HTMLInputElement).checked"
        />
        <span class="min-w-0">
          <span class="block text-sm font-medium">{{ t('linkTemplate.setDefault') }}</span>
          <span class="block text-xs text-muted-foreground">{{ t('linkTemplate.setDefaultHint') }}</span>
        </span>
      </label>

      <p v-if="availableTemplates.length === 0" class="text-sm text-muted-foreground">
        {{ t('linkExisting.allLinked') }}
      </p>

      <div class="flex justify-end gap-2 pt-1">
        <Button type="button" variant="outline" @click="open = false">{{ t('common.cancel') }}</Button>
        <Button type="submit" :disabled="availableTemplates.length === 0">
          <Link2 class="size-4" />
          {{ t('linkTemplate.submit') }}
        </Button>
      </div>
    </form>
  </BaseModal>
</template>

<script setup lang="ts">
import { Link2, TriangleAlert } from '@lucide/vue'
import { computed, ref, watch } from 'vue'

import BaseModal from '@/components/admin/BaseModal.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { AgentTemplate, ProviderInstance } from '@/domain/admin'

const props = defineProps<{
  provider?: ProviderInstance
  /** Templates are global resources, so every one of them is a valid target. */
  templates: AgentTemplate[]
  providerNameById: (id?: string) => string
}>()

const open = defineModel<boolean>({ required: true })
const emit = defineEmits<{ link: [templateId: string] }>()

const { t, providerTypeLabel } = useI18n()

const templateId = ref('')

const selectedTemplate = computed(() => props.templates.find((item) => item.id === templateId.value))

/**
 * A template holds one provider per type, so linking over an occupied slot is a
 * replacement. Surfaced instead of applied silently.
 */
const existingBindingId = computed(() => {
  if (!props.provider || !selectedTemplate.value) return undefined
  return selectedTemplate.value.providerBindings[props.provider.type]
})

const alreadyLinked = computed(() => existingBindingId.value === props.provider?.id)

const replacing = computed(() => Boolean(existingBindingId.value) && !alreadyLinked.value)

watch(
  () => [open.value, props.provider] as const,
  () => {
    if (!open.value) return
    templateId.value = props.templates[0]?.id ?? ''
  },
  { immediate: true },
)

function link() {
  if (!props.provider || !templateId.value) return
  emit('link', templateId.value)
  open.value = false
}
</script>

<template>
  <BaseModal
    v-model="open"
    :title="t('providerLink.title', { name: provider?.name ?? '' })"
    :description="t('providerLink.description')"
  >
    <form class="space-y-4" @submit.prevent="link">
      <label class="block space-y-1.5">
        <span class="text-sm font-medium">{{ t('providerLink.template') }}</span>
        <select v-model="templateId" class="admin-input" required :disabled="!templates.length">
          <option v-for="template in templates" :key="template.id" :value="template.id">
            {{ template.name }} · {{ template.language || '—' }}
          </option>
        </select>
      </label>

      <dl class="rounded-lg border border-border/70 bg-surface px-3.5 py-3 text-sm">
        <div class="flex items-center justify-between gap-3 py-1">
          <dt class="text-xs text-muted-foreground">{{ t('providerLink.providerRole') }}</dt>
          <dd class="font-medium">{{ provider ? providerTypeLabel(provider.type) : '—' }}</dd>
        </div>
      </dl>

      <div
        v-if="provider && selectedTemplate"
        class="flex gap-2 rounded-md border border-border/70 px-3 py-2.5 text-xs leading-relaxed text-muted-foreground"
      >
        <TriangleAlert v-if="replacing" class="mt-0.5 size-3.5 shrink-0 text-danger" aria-hidden="true" />
        <span v-if="alreadyLinked">{{ t('providerLink.alreadyLinked') }}</span>
        <span v-else-if="replacing">
          <span class="font-medium text-foreground">
            {{ t('providerLink.replacesTitle', { type: providerTypeLabel(provider.type) }) }}
          </span>
          {{
            t('providerLink.replaces', {
              template: selectedTemplate.name,
              provider: providerNameById(existingBindingId),
              type: providerTypeLabel(provider.type),
              name: provider.name,
            })
          }}
        </span>
        <span v-else>
          {{ t('providerLink.missingSlot', { template: selectedTemplate.name, type: providerTypeLabel(provider.type) }) }}
        </span>
      </div>

      <p v-else-if="provider" class="rounded-md bg-muted/60 px-3 py-2.5 text-xs text-muted-foreground">
        {{ t('providerLink.noTemplates') }}
      </p>

      <div class="flex justify-end gap-2 pt-1">
        <Button type="button" variant="outline" @click="open = false">{{ t('common.cancel') }}</Button>
        <Button
          type="submit"
          :disabled="!templateId || alreadyLinked"
          :class="replacing ? 'bg-danger text-white hover:bg-danger/90' : undefined"
        >
          <Link2 class="size-4" />
          {{ replacing ? t('providerLink.replaceSubmit') : t('providerLink.submit') }}
        </Button>
      </div>
    </form>
  </BaseModal>
</template>
<script setup lang="ts">
import { TriangleAlert } from '@lucide/vue'
import { computed, reactive, useId, watch } from 'vue'

import BaseModal from '@/components/admin/BaseModal.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import {
  providerTypes,
  type ProviderInstance,
  type ProviderStatus,
  type ProviderType,
} from '@/domain/admin'

const props = withDefaults(
  defineProps<{
    provider?: ProviderInstance
    /** Preselects the type when the action was scoped, e.g. "Create TTS provider". */
    initialType?: ProviderType
    /** Adapters already in the catalog, used to suggest one valid for the chosen type. */
    providers?: ProviderInstance[]
    /** Templates bound to the provider being edited, to state the blast radius. */
    usageCount?: number
  }>(),
  { initialType: 'llm', providers: () => [], usageCount: 0 },
)

const open = defineModel<boolean>({ required: true })
const emit = defineEmits<{
  save: [payload: { name: string; type: ProviderType; adapter: string; model: string; description: string; status: ProviderStatus; endpoint?: string }]
}>()

const { t, providerTypeLabel } = useI18n()

const adapterListId = useId()

const form = reactive({
  name: '',
  type: 'llm' as ProviderType,
  adapter: '',
  model: '',
  description: '',
  status: 'ready' as ProviderStatus,
  endpoint: '',
})

/**
 * Adapters already used by providers of the selected type. There is no adapter
 * registry in the mock domain yet, so this suggests rather than restricts.
 */
const adapterHints = computed(() => [
  ...new Set(
    props.providers
      .filter((item) => item.type === form.type)
      .map((item) => item.adapter)
      .filter(Boolean),
  ),
])

const typeChanged = (next: ProviderType) => {
  if (!adapterHints.value.includes(form.adapter)) form.adapter = ''
}

watch(
  () => [open.value, props.provider] as const,
  () => {
    if (!open.value) return
    form.name = props.provider?.name ?? ''
    form.type = props.provider?.type ?? props.initialType
    form.adapter = props.provider?.adapter ?? ''
    form.model = props.provider?.model ?? ''
    form.description = props.provider?.description ?? ''
    form.status = props.provider?.status ?? 'ready'
    form.endpoint = props.provider?.endpoint ?? ''
  },
  { immediate: true },
)

function submit() {
  if (!form.name.trim() || !form.adapter.trim()) return
  emit('save', {
    name: form.name.trim(),
    type: form.type,
    adapter: form.adapter.trim(),
    model: form.model.trim(),
    description: form.description.trim(),
    status: form.status,
    endpoint: form.endpoint.trim() || undefined,
  })
  open.value = false
}
</script>

<template>
  <BaseModal
    v-model="open"
    :title="provider ? t('providers.editTitle') : t('providers.createTitle')"
    :description="provider ? t('providers.editDescription') : t('providers.createDescription')"
  >
    <form class="grid gap-4 sm:grid-cols-2" @submit.prevent="submit">
      <div
        v-if="provider && usageCount > 0"
        class="flex gap-2 rounded-md border border-warning/40 bg-warning/10 px-3 py-2.5 text-xs leading-relaxed text-muted-foreground sm:col-span-2"
      >
        <TriangleAlert class="mt-0.5 size-3.5 shrink-0 text-warning" aria-hidden="true" />
        <span>{{ t('providers.usedBy', { count: usageCount }) }}</span>
      </div>

      <label class="block space-y-1.5">
        <span class="text-sm font-medium">{{ t('providers.type') }}</span>
        <select
          v-model="form.type"
          class="admin-input"
          :disabled="Boolean(provider)"
          @change="typeChanged(form.type)"
        >
          <option v-for="type in providerTypes" :key="type" :value="type">
            {{ providerTypeLabel(type) }}
          </option>
        </select>
      </label>

      <label class="block space-y-1.5">
        <span class="text-sm font-medium">{{ t('providers.name') }}</span>
        <input v-model="form.name" class="admin-input" :placeholder="t('providers.namePlaceholder')" required />
      </label>

      <label class="block space-y-1.5">
        <span class="text-sm font-medium">{{ t('providers.adapter') }}</span>
        <input
          v-model="form.adapter"
          class="admin-input font-mono text-sm"
          :list="adapterListId"
          required
        />
        <datalist :id="adapterListId">
          <option v-for="adapter in adapterHints" :key="adapter" :value="adapter" />
        </datalist>
      </label>

      <label v-if="form.type !== 'speaker'" class="block space-y-1.5">
        <span class="text-sm font-medium">{{ t('providers.model') }}</span>
        <input v-model="form.model" class="admin-input font-mono text-sm" />
      </label>

      <label class="block space-y-1.5">
        <span class="text-sm font-medium">{{ t('providers.status') }}</span>
        <select v-model="form.status" class="admin-input">
          <option value="ready">{{ t('status.provider.ready') }}</option>
          <option value="disabled">{{ t('status.provider.disabled') }}</option>
          <option value="error">{{ t('status.provider.error') }}</option>
        </select>
      </label>

      <label v-if="form.type !== 'speaker'" class="block space-y-1.5">
        <span class="text-sm font-medium">{{ t('providers.endpoint') }}</span>
        <input
          v-model="form.endpoint"
          class="admin-input font-mono text-sm"
          :placeholder="t('common.optional')"
        />
      </label>

      <label v-if="form.type !== 'speaker'" class="block space-y-1.5 sm:col-span-2">
        <span class="text-sm font-medium">{{ t('providers.descriptionField') }}</span>
        <textarea v-model="form.description" class="admin-textarea min-h-24" />
      </label>

      <div class="flex justify-end gap-2 pt-2 sm:col-span-2">
        <Button type="button" variant="outline" @click="open = false">{{ t('common.cancel') }}</Button>
        <Button type="submit">
          {{ provider ? t('common.saveChanges') : t('providers.create') }}
        </Button>
      </div>
    </form>
  </BaseModal>
</template>

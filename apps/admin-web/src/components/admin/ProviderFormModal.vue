<script setup lang="ts">
import { TriangleAlert } from '@lucide/vue'
import { computed, reactive, ref, useId, watch } from 'vue'

import { providerAdaptersApi } from '@/api/provider-adapters'
import type { ProviderAdapter } from '@/api/types/providers'
import ProviderConfigEditor from '@/components/providers/ProviderConfigEditor.vue'
import ProviderTestPanel from '@/components/providers/ProviderTestPanel.vue'
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
  save: [payload: { name: string; type: ProviderType; adapter: string; model: string; description: string; status: ProviderStatus; endpoint?: string; apiKey?: string; configJson?: Record<string, unknown> }]
}>()

const { t, providerTypeLabel } = useI18n()

const descriptor = ref<ProviderAdapter>()
const configError = ref('')
let descriptorVersion = 0
const adapterListId = useId()

const form = reactive({
  name: '',
  type: 'llm' as ProviderType,
  adapter: '',
  model: '',
  description: '',
  status: 'ready' as ProviderStatus,
  endpoint: '',
  apiKey: '',
  configJson: {} as Record<string, unknown>,
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

watch(() => form.adapter, (adapter, previous) => {
  form.apiKey = ''
  if (previous && adapter !== props.provider?.adapter) form.configJson = {}
})
watch([open, () => form.adapter], async ([isOpen, adapter]) => {
  const version = ++descriptorVersion
  descriptor.value = undefined; configError.value = ''
  if (!isOpen || !adapter) return
  try { const result = await providerAdaptersApi.get(adapter); if (version === descriptorVersion) descriptor.value = result }
  catch { if (version === descriptorVersion) configError.value = t('diagnostics.configUnavailable') }
})

watch(
  () => [open.value, props.provider] as const,
  () => {
    form.apiKey = ''
    if (!open.value) return
    form.name = props.provider?.name ?? ''
    form.type = props.provider?.type ?? props.initialType
    form.adapter = props.provider?.adapter ?? ''
    form.model = props.provider?.model ?? ''
    form.description = props.provider?.description ?? ''
    form.status = props.provider?.status ?? 'ready'
    form.endpoint = props.provider?.endpoint ?? ''
    form.configJson = { ...props.provider?.configJson }
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
    configJson: { ...form.configJson },
    ...(form.apiKey && ['openai', 'chillaudio_ws'].includes(form.adapter) ? { apiKey: form.apiKey } : {}),
  })
  form.apiKey = ''
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

      <label class="block space-y-1.5">
        <span class="text-sm font-medium">{{ t('providers.status') }}</span>
        <select v-model="form.status" class="admin-input">
          <option value="ready">{{ t('status.provider.ready') }}</option>
          <option value="disabled">{{ t('status.provider.disabled') }}</option>
          <option value="error">{{ t('status.provider.error') }}</option>
        </select>
      </label>

      <div class="sm:col-span-2"><p v-if="configError" class="text-sm text-danger">{{ configError }}</p><ProviderConfigEditor v-model="form.configJson" :schema="descriptor?.config_schema" /></div>

      <label v-if="['openai', 'chillaudio_ws'].includes(form.adapter)" class="block space-y-1.5 sm:col-span-2">
        <span class="text-sm font-medium">API key / token mới (tùy chọn)</span>
        <p v-if="provider?.credential" class="text-xs text-muted-foreground">Key hiện tại: {{ provider.credential.masked_key }}</p>
        <input v-model="form.apiKey" type="password" autocomplete="new-password" maxlength="4096" class="admin-input" />
        <small class="block text-xs text-muted-foreground">Để trống để giữ key hiện tại. Key mới được mã hóa trên server.</small>
      </label>

      <ProviderTestPanel v-if="open && ['asr','llm','tts'].includes(form.type)" class="sm:col-span-2" :type="form.type" :adapter="form.adapter" :capabilities="descriptor?.capabilities" :draft="{ type: form.type as 'asr' | 'llm' | 'tts', adapter: form.adapter, config_json: form.configJson, ...(form.apiKey ? { api_key: form.apiKey } : provider && ['openai', 'chillaudio_ws'].includes(form.adapter) && form.adapter === provider.adapter && provider.desiredRevision ? { saved_credential: { key: provider.id, expected_revision: provider.desiredRevision } } : {}) }" />

      <div class="flex justify-end gap-2 pt-2 sm:col-span-2">
        <Button type="button" variant="outline" @click="open = false">{{ t('common.cancel') }}</Button>
        <Button type="submit">
          {{ provider ? t('common.saveChanges') : t('providers.create') }}
        </Button>
      </div>
    </form>
  </BaseModal>
</template>

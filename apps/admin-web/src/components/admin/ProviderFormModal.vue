<script setup lang="ts">
import { TriangleAlert } from '@lucide/vue'
import { computed, reactive, ref, useId, watch } from 'vue'

import { llmGatewayApi } from '@/api/llm-gateway'
import { providerAdaptersApi } from '@/api/provider-adapters'
import type {
  GatewayAccountView,
  GatewayPoolModelView,
  GatewayVendorInfo,
} from '@/api/types/llm-gateway'
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

const isReadOnly = computed(() => props.provider?.source === 'config')

const open = defineModel<boolean>({ required: true })
const emit = defineEmits<{
  save: [payload: { name: string; type: ProviderType; adapter: string; model: string; description: string; status: ProviderStatus; endpoint?: string; apiKey?: string | null; configJson?: Record<string, unknown> }]
}>()

const { t, providerTypeLabel } = useI18n()

const descriptor = ref<ProviderAdapter>()
const configError = ref('')
let descriptorVersion = 0
const adapterListId = useId()

type AuthSourceMode = 'static_api_key' | 'gateway_account' | 'gateway_pool'
const authSource = ref<AuthSourceMode>('static_api_key')
const gatewayVendors = ref<GatewayVendorInfo[]>([])
const gatewayAccounts = ref<GatewayAccountView[]>([])

const supportsGatewayBinding = computed(
  () => form.adapter === 'openai' || form.adapter === 'llm_gateway',
)

const isUsingGateway = computed(
  () =>
    form.adapter === 'llm_gateway' ||
    (supportsGatewayBinding.value && authSource.value !== 'static_api_key'),
)

const KNOWN_GATEWAY_VENDORS = [
  'anthropic',
  'openai_codex',
  'gemini',
  'github_copilot',
  'muse',
] as const

function resolveVendorForBinding(binding: string): string {
  if (binding.startsWith('account:')) {
    const key = binding.slice('account:'.length)
    const found = gatewayAccounts.value.find((a) => a.account_key === key)?.vendor
    if (found) return found
    for (const v of KNOWN_GATEWAY_VENDORS) {
      if (key.startsWith(`gw_${v}_`)) return v
    }
    return ''
  }
  if (binding.startsWith('pool:')) {
    return binding.slice('pool:'.length)
  }
  return ''
}

function poolModelsForVendor(vendorId: string): GatewayPoolModelView[] {
  if (!vendorId) return []
  const vendorInfo = gatewayVendors.value.find((v) => v.vendor === vendorId)
  const seen = new Set<string>()
  return (vendorInfo?.models ?? []).filter((m) => {
    const id = m.model?.trim()
    if (!id || seen.has(id)) return false
    seen.add(id)
    return true
  })
}

const currentVendorPoolModels = computed<GatewayPoolModelView[]>(() => {
  if (!isUsingGateway.value) return []
  const binding =
    typeof form.configJson.gateway_binding === 'string'
      ? form.configJson.gateway_binding.trim()
      : ''
  return poolModelsForVendor(resolveVendorForBinding(binding))
})

const filteredSchema = computed(() => {
  if (!descriptor.value?.config_schema) return undefined
  if (!supportsGatewayBinding.value) return descriptor.value.config_schema
  return {
    ...descriptor.value.config_schema,
    fields: (descriptor.value.config_schema.fields ?? [])
      .filter((f) => f.key !== 'gateway_binding')
      .map((f) =>
        f.key === 'model' && isUsingGateway.value
          ? { ...f, type: 'select' as const, enum_source: 'models' as const }
          : f,
      ),
  }
})

const effectiveCapabilities = computed(() => {
  const base = descriptor.value?.capabilities
  if (!isUsingGateway.value) return base
  return {
    ...base,
    models: currentVendorPoolModels.value.map((m) => ({
      id: m.model,
      name:
        m.display_name && m.display_name !== m.model
          ? `${m.display_name} (${m.model})`
          : m.model,
    })),
  }
})

async function loadGatewayData() {
  try {
    const [vendorRes, accountRes] = await Promise.all([
      llmGatewayApi.listVendors(),
      llmGatewayApi.listAccounts(),
    ])
    gatewayVendors.value = vendorRes.vendors
    gatewayAccounts.value = accountRes.accounts
    if (isUsingGateway.value && !isReadOnly.value) {
      const currentBinding =
        typeof form.configJson.gateway_binding === 'string'
          ? form.configJson.gateway_binding.trim()
          : ''
      if (currentBinding) {
        updateGatewayBinding(currentBinding)
      }
    }
  } catch {
    /* Optional gateway lookup. */
  }
}

function applyPoolModelDefaultsToObject(
  target: Record<string, unknown>,
  poolModel: GatewayPoolModelView,
) {
  target.model = poolModel.model
  if (Array.isArray(poolModel.tags) && poolModel.tags.length > 0) {
    target.tags = [...poolModel.tags]
  } else {
    delete target.tags
  }
  if (poolModel.max_tokens !== undefined) {
    target.max_tokens = poolModel.max_tokens
  }
  if (poolModel.context_limit_tokens !== undefined) {
    target.context_limit_tokens = poolModel.context_limit_tokens
  }
  if (poolModel.reasoning_effort) {
    target.reasoning_effort = poolModel.reasoning_effort
  }
  if (poolModel.temperature_percent !== undefined) {
    target.temperature_percent = poolModel.temperature_percent
  }
  if (poolModel.timeout_ms !== undefined) {
    target.timeout_ms = poolModel.timeout_ms
  }
}

function updateGatewayBinding(binding: string) {
  if (isReadOnly.value) return
  configError.value = ''
  const next = { ...form.configJson }
  if (!binding) {
    delete next.gateway_binding
    form.configJson = next
    return
  }
  next.gateway_binding = binding
  const vendorId = resolveVendorForBinding(binding)
  const poolModels = poolModelsForVendor(vendorId)
  const currentModel = typeof next.model === 'string' ? next.model.trim() : ''
  if (poolModels.length > 0) {
    if (!poolModels.some((m) => m.model === currentModel)) {
      applyPoolModelDefaultsToObject(next, poolModels[0]!)
    }
  } else if (gatewayVendors.value.length > 0) {
    delete next.model
    delete next.tags
  }
  if (form.adapter === 'openai') {
    const currentBaseUrl = typeof next.base_url === 'string' ? next.base_url.trim() : ''
    if (!currentBaseUrl) {
      const vendorInfo = gatewayVendors.value.find((v) => v.vendor === vendorId)
      if (vendorInfo?.default_base_url) {
        next.base_url = vendorInfo.default_base_url
      }
    }
  }
  form.configJson = next
}

function setAuthSource(mode: AuthSourceMode) {
  if (isReadOnly.value) return
  authSource.value = mode
  configError.value = ''
  if (mode === 'static_api_key') {
    const next = { ...form.configJson }
    delete next.gateway_binding
    form.configJson = next
    return
  }
  form.apiKey = ''
  const current =
    typeof form.configJson.gateway_binding === 'string' ? form.configJson.gateway_binding : ''
  if (mode === 'gateway_pool' && !current.startsWith('pool:')) {
    updateGatewayBinding(gatewayVendors.value[0]?.pool_binding ?? 'pool:openai_codex')
  } else if (mode === 'gateway_account' && !current.startsWith('account:')) {
    const firstKey = gatewayAccounts.value[0]?.account_key
    updateGatewayBinding(firstKey ? `account:${firstKey}` : '')
  }
}

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
  if (isReadOnly.value) return
  if (!adapterHints.value.includes(form.adapter)) form.adapter = ''
}

watch(() => form.adapter, (adapter, previous) => {
  if (isReadOnly.value) return
  form.apiKey = ''
  if (previous && adapter !== props.provider?.adapter) form.configJson = {}
})
watch(
  () => form.configJson.model,
  (nextModel, prevModel) => {
    if (isReadOnly.value || !isUsingGateway.value) return
    if (!prevModel || nextModel === prevModel || typeof nextModel !== 'string') return
    const found = currentVendorPoolModels.value.find((m) => m.model === nextModel)
    if (found) {
      const next = { ...form.configJson }
      applyPoolModelDefaultsToObject(next, found)
      form.configJson = next
    }
  },
)
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
    const binding =
      typeof form.configJson.gateway_binding === 'string'
        ? form.configJson.gateway_binding.trim()
        : ''
    if (binding.startsWith('account:')) {
      authSource.value = 'gateway_account'
    } else if (binding.startsWith('pool:') || form.adapter === 'llm_gateway') {
      authSource.value = 'gateway_pool'
    } else {
      authSource.value = 'static_api_key'
    }
    if (form.adapter === 'openai' || form.adapter === 'llm_gateway') {
      void loadGatewayData()
    }
  },
  { immediate: true },
)

function submit() {
  if (isReadOnly.value) return
  if (!form.name.trim() || !form.adapter.trim()) return
  const nextConfig = { ...form.configJson }
  if (supportsGatewayBinding.value && authSource.value === 'static_api_key') {
    delete nextConfig.gateway_binding
  }
  const usingGateway =
    form.adapter === 'llm_gateway' ||
    (supportsGatewayBinding.value && authSource.value !== 'static_api_key')
  if (
    usingGateway &&
    (typeof nextConfig.gateway_binding !== 'string' || !nextConfig.gateway_binding.trim())
  ) {
    configError.value = t('llmGateway.auth.bindingRequired')
    return
  }
  if (usingGateway && gatewayVendors.value.length > 0) {
    const selectedModel = typeof nextConfig.model === 'string' ? nextConfig.model.trim() : ''
    if (
      currentVendorPoolModels.value.length === 0 ||
      !selectedModel ||
      !currentVendorPoolModels.value.some((m) => m.model === selectedModel)
    ) {
      configError.value =
        currentVendorPoolModels.value.length === 0
          ? t('llmGateway.pool.noModels')
          : t('llmGateway.modelModal.modelRequired')
      return
    }
  }
  configError.value = ''
  emit('save', {
    name: form.name.trim(),
    type: form.type,
    adapter: form.adapter.trim(),
    model: form.model.trim(),
    description: form.description.trim(),
    status: form.status,
    endpoint: form.endpoint.trim() || undefined,
    configJson: nextConfig,
    ...(usingGateway
      ? { apiKey: null }
      : form.apiKey && ['openai', 'chillaudio_ws'].includes(form.adapter)
        ? { apiKey: form.apiKey }
        : {}),
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
        <input :disabled="isReadOnly" v-model="form.name" class="admin-input" :placeholder="t('providers.namePlaceholder')" required />
      </label>

      <label class="block space-y-1.5">
        <span class="text-sm font-medium">{{ t('providers.adapter') }}</span>
        <input :disabled="isReadOnly"
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
        <select :disabled="isReadOnly" v-model="form.status" class="admin-input">
          <option value="ready">{{ t('status.provider.ready') }}</option>
          <option value="disabled">{{ t('status.provider.disabled') }}</option>
          <option value="error">{{ t('status.provider.error') }}</option>
        </select>
      </label>

      <div class="sm:col-span-2"><p v-if="configError" class="text-sm text-danger">{{ configError }}</p><ProviderConfigEditor :readonly="isReadOnly" v-model="form.configJson" :schema="filteredSchema" :capabilities="effectiveCapabilities" /></div>

      <div v-if="supportsGatewayBinding" class="space-y-2.5 rounded-lg border border-border/70 bg-surface/50 p-3.5 sm:col-span-2">
        <span class="block text-sm font-medium">{{ t('llmGateway.auth.title') }}</span>
        <div class="flex flex-wrap gap-2">
          <button
            v-if="form.adapter === 'openai'"
            type="button"
            class="rounded-md border px-3 py-1 text-xs font-medium transition-colors"
            :class="
              authSource === 'static_api_key'
                ? 'border-studio-violet bg-studio-violet text-white'
                : 'border-border bg-background text-muted-foreground hover:bg-accent'
            "
            :disabled="isReadOnly"
            @click="setAuthSource('static_api_key')"
          >
            {{ t('llmGateway.auth.staticApiKey') }}
          </button>
          <button
            type="button"
            class="rounded-md border px-3 py-1 text-xs font-medium transition-colors"
            :class="
              authSource === 'gateway_account'
                ? 'border-studio-violet bg-studio-violet text-white'
                : 'border-border bg-background text-muted-foreground hover:bg-accent'
            "
            :disabled="isReadOnly"
            @click="setAuthSource('gateway_account')"
          >
            {{ t('llmGateway.auth.gatewayAccount') }}
          </button>
          <button
            type="button"
            class="rounded-md border px-3 py-1 text-xs font-medium transition-colors"
            :class="
              authSource === 'gateway_pool'
                ? 'border-studio-violet bg-studio-violet text-white'
                : 'border-border bg-background text-muted-foreground hover:bg-accent'
            "
            :disabled="isReadOnly"
            @click="setAuthSource('gateway_pool')"
          >
            {{ t('llmGateway.auth.gatewayPool') }}
          </button>
        </div>

        <label v-if="authSource === 'gateway_account'" class="block space-y-1">
          <span class="text-xs text-muted-foreground">{{ t('llmGateway.auth.gatewayAccount') }}</span>
          <select
            :disabled="isReadOnly"
            class="admin-input font-mono text-xs"
            :value="String(form.configJson.gateway_binding ?? '')"
            @change="updateGatewayBinding(($event.target as HTMLSelectElement).value)"
          >
            <option value="">{{ t('llmGateway.auth.selectAccount') }}</option>
            <option
              v-for="acc in gatewayAccounts"
              :key="acc.account_key"
              :value="`account:${acc.account_key}`"
            >
              {{ acc.label }} ({{ acc.vendor }} — {{ acc.account_email || acc.account_key }})
            </option>
          </select>
        </label>

        <label v-else-if="authSource === 'gateway_pool'" class="block space-y-1">
          <span class="text-xs text-muted-foreground">{{ t('llmGateway.auth.gatewayPool') }}</span>
          <select
            :disabled="isReadOnly"
            class="admin-input font-mono text-xs"
            :value="String(form.configJson.gateway_binding ?? '')"
            @change="updateGatewayBinding(($event.target as HTMLSelectElement).value)"
          >
            <option value="">{{ t('llmGateway.auth.selectPool') }}</option>
            <option
              v-for="v in gatewayVendors"
              :key="v.vendor"
              :value="v.pool_binding"
            >
              {{ v.display_name }} ({{ v.pool_binding }} — {{ v.active_accounts }}/{{ v.total_accounts }} active)
            </option>
          </select>
        </label>
      </div>

      <label v-if="(['openai', 'chillaudio_ws'].includes(form.adapter)) && (!supportsGatewayBinding || authSource === 'static_api_key')" class="block space-y-1.5 sm:col-span-2">
        <span class="text-sm font-medium">API key / token mới (tùy chọn)</span>
        <p v-if="provider?.credential" class="text-xs text-muted-foreground">Key hiện tại: {{ provider.credential.masked_key }}</p>
        <input :disabled="isReadOnly" v-model="form.apiKey" type="password" autocomplete="new-password" maxlength="4096" class="admin-input" />
        <small class="block text-xs text-muted-foreground">Để trống để giữ key hiện tại. Key mới được mã hóa trên server.</small>
      </label>

      <ProviderTestPanel v-if="open && ['asr','llm','tts'].includes(form.type)" class="sm:col-span-2" :type="form.type" :adapter="form.adapter" :model="typeof form.configJson.model === 'string' && form.configJson.model ? form.configJson.model : form.model" :capabilities="descriptor?.capabilities" :saved-key="isReadOnly ? provider?.id : undefined" :draft="isReadOnly ? undefined : { type: form.type as 'asr' | 'llm' | 'tts', adapter: form.adapter, config_json: form.configJson, ...(form.apiKey ? { api_key: form.apiKey } : provider && ['openai', 'chillaudio_ws'].includes(form.adapter) && form.adapter === provider.adapter && provider.desiredRevision ? { saved_credential: { key: provider.id, expected_revision: provider.desiredRevision } } : {}) }" />

      <div class="flex justify-end gap-2 pt-2 sm:col-span-2">
        <Button type="button" variant="outline" @click="open = false">{{ t('common.cancel') }}</Button>
        <Button type="submit" :disabled="isReadOnly">
          {{ provider ? t('common.saveChanges') : t('providers.create') }}
        </Button>
      </div>
    </form>
  </BaseModal>
</template>

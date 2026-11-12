<script setup lang="ts">
import { ArrowDown, ArrowUpRight, Check, FlaskConical, KeyRound, Layers, LockKeyhole, SlidersHorizontal, TriangleAlert } from '@lucide/vue'
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
import { providerTypeIcons } from '@/lib/providerTypeIcons'

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

const { t, providerTypeLabel, providerStatusLabel } = useI18n()

const descriptor = ref<ProviderAdapter>()
const configError = ref('')
let descriptorVersion = 0
const adapterListId = useId()
const formId = useId()
const identitySection = ref<HTMLElement>()
const connectionSection = ref<HTMLElement>()
const configurationSection = ref<HTMLElement>()
const diagnosticSection = ref<HTMLElement>()

function jumpTo(section: HTMLElement | undefined) {
  section?.scrollIntoView({ behavior: window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth', block: 'start' })
  section?.focus({ preventScroll: true })
}

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

const CUSTOM_TIMEOUT_FIELDS = new Set(['first_byte_ms', 'idle_ms', 'max_total_ms'])

const filteredSchema = computed(() => {
  if (!descriptor.value?.config_schema) return undefined
  const isCustomTimeout = form.configJson.timeout_profile === 'custom'
  return {
    ...descriptor.value.config_schema,
    fields: (descriptor.value.config_schema.fields ?? [])
      .filter((f) => {
        if (supportsGatewayBinding.value && f.key === 'gateway_binding') return false
        if (CUSTOM_TIMEOUT_FIELDS.has(f.key) && !isCustomTimeout) return false
        return true
      })
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

const selectedModel = computed(() =>
  typeof form.configJson.model === 'string' && form.configJson.model
    ? form.configJson.model
    : form.model,
)
const hasDiagnostic = computed(() => ['asr', 'llm', 'tts'].includes(form.type))
const authLabel = computed(() => {
  if (isUsingGateway.value) return t(authSource.value === 'gateway_account' ? 'llmGateway.auth.gatewayAccount' : 'llmGateway.auth.gatewayPool')
  return t('providers.studio.direct')
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
  if (nextConfig.timeout_profile && nextConfig.timeout_profile !== 'custom') {
    delete nextConfig.first_byte_ms
    delete nextConfig.idle_ms
    delete nextConfig.max_total_ms
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
    width-class="max-w-[1120px] h-[min(860px,90dvh)]"
    body-class="provider-modal-body"
  >
    <div class="provider-studio" :data-provider-type="form.type">
      <aside class="provider-passport">
        <div class="passport-sticky">
          <div class="passport-eyebrow"><span class="passport-marker" /> PROVIDER STUDIO <span>/ {{ form.type.toUpperCase() }}</span></div>
          <div class="provider-emblem" aria-hidden="true">
            <svg viewBox="0 0 240 154" fill="none">
              <path d="M0 77h51m138 0h51M120 0v22m0 110v22" class="emblem-axis" />
              <ellipse cx="120" cy="77" rx="93" ry="44" transform="rotate(-25 120 77)" />
              <ellipse cx="120" cy="77" rx="93" ry="44" transform="rotate(25 120 77)" />
              <circle cx="120" cy="77" r="58" stroke-dasharray="2 7" />
              <circle cx="38" cy="104" r="3" class="emblem-node" />
              <circle cx="202" cy="50" r="3" class="emblem-node" />
            </svg>
            <span><component :is="providerTypeIcons[form.type]" :size="31" :stroke-width="1.4" /></span>
          </div>
          <p class="passport-role">{{ providerTypeLabel(form.type) }}</p>
          <h3 class="passport-name">{{ form.name || t('providers.studio.untitled') }}</h3>
          <p class="passport-caption">{{ t('providers.studio.caption') }}</p>
          <dl class="passport-specs">
            <div><dt>{{ t('providers.adapter') }}</dt><dd>{{ form.adapter || '—' }}</dd></div>
            <div><dt>{{ t('providers.model') }}</dt><dd>{{ selectedModel || '—' }}</dd></div>
            <div><dt>{{ t('providers.status') }}</dt><dd class="passport-status" :data-status="form.status"><span />{{ providerStatusLabel(form.status) }}</dd></div>
          </dl>
          <nav class="passport-nav" :aria-label="t('providers.studio.navigation')">
            <button type="button" @click="jumpTo(identitySection)"><span>01</span>{{ t('providers.studio.identity') }}<ArrowDown :size="13" /></button>
            <button type="button" @click="jumpTo(connectionSection)"><span>02</span>{{ t('providers.studio.connection') }}<ArrowDown :size="13" /></button>
            <button type="button" @click="jumpTo(configurationSection)"><span>03</span>{{ t('providers.studio.parameters') }}<ArrowDown :size="13" /></button>
            <button v-if="hasDiagnostic" type="button" @click="jumpTo(diagnosticSection)"><span>04</span>{{ t('providers.testAction') }}<ArrowDown :size="13" /></button>
          </nav>
          <div class="passport-note" :class="{ 'has-usage': usageCount > 0 }">
            <LockKeyhole v-if="isReadOnly" :size="16" />
            <TriangleAlert v-else-if="usageCount > 0" :size="16" />
            <Layers v-else :size="16" />
            <p>{{ isReadOnly ? t('providers.studio.readOnly') : usageCount > 0 ? t('providers.usedBy', { count: usageCount }) : t('providers.unused') }}</p>
          </div>
        </div>
      </aside>

      <form :id="formId" class="provider-workbench" @submit.prevent="submit">
        <section ref="identitySection" tabindex="-1" class="provider-section">
          <header class="provider-section-heading">
            <span class="section-number">01</span>
            <div><h3>{{ t('providers.studio.identity') }}</h3><p>{{ t('providers.studio.identityHint') }}</p></div>
            <component :is="providerTypeIcons[form.type]" :size="18" aria-hidden="true" />
          </header>
          <div class="grid gap-4 sm:grid-cols-2">
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
              <input :disabled="Boolean(provider)"
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

          </div>
        </section>

        <section ref="connectionSection" tabindex="-1" class="provider-section">
          <header class="provider-section-heading">
            <span class="section-number">02</span>
            <div><h3>{{ t('providers.studio.connection') }}</h3><p>{{ authLabel }}</p></div>
            <KeyRound :size="18" aria-hidden="true" />
          </header>
          <div v-if="supportsGatewayBinding" class="provider-auth space-y-3">
            <span class="block text-sm font-medium">{{ t('llmGateway.auth.title') }}</span>
            <div class="flex flex-wrap gap-2">
              <button
                v-if="form.adapter === 'openai'"
                type="button"
                class="provider-auth-option"
                :class="
                  authSource === 'static_api_key'
                    ? 'is-selected'
                    : ''
                "
                :disabled="isReadOnly"
                :aria-pressed="authSource === 'static_api_key'"
                @click="setAuthSource('static_api_key')"
              >
                {{ t('llmGateway.auth.staticApiKey') }}
              </button>
              <button
                type="button"
                class="provider-auth-option"
                :class="
                  authSource === 'gateway_account'
                    ? 'is-selected'
                    : ''
                "
                :disabled="isReadOnly"
                :aria-pressed="authSource === 'gateway_account'"
                @click="setAuthSource('gateway_account')"
              >
                {{ t('llmGateway.auth.gatewayAccount') }}
              </button>
              <button
                type="button"
                class="provider-auth-option"
                :class="
                  authSource === 'gateway_pool'
                    ? 'is-selected'
                    : ''
                "
                :disabled="isReadOnly"
                :aria-pressed="authSource === 'gateway_pool'"
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
            <span class="text-sm font-medium">{{ t('providers.studio.newKey') }}</span>
            <p v-if="provider?.credential" class="text-xs text-muted-foreground">{{ t('providers.studio.currentKey') }} {{ provider.credential.masked_key }}</p>
            <input :disabled="isReadOnly" v-model="form.apiKey" type="password" autocomplete="new-password" maxlength="4096" class="admin-input" />
            <small class="block text-xs text-muted-foreground">{{ t('providers.studio.keyHint') }}</small>
          </label>

          <p v-if="!supportsGatewayBinding && !['openai', 'chillaudio_ws'].includes(form.adapter)" class="provider-local-note"><LockKeyhole :size="16" aria-hidden="true" />{{ t('providers.studio.adapterAuth') }}</p>
        </section>

        <section ref="configurationSection" tabindex="-1" class="provider-section">
          <header class="provider-section-heading">
            <span class="section-number">03</span>
            <div><h3>{{ t('providers.studio.parameters') }}</h3><p>{{ t('providers.studio.parametersHint') }}</p></div>
            <SlidersHorizontal :size="18" aria-hidden="true" />
          </header>
          <p v-if="configError" role="alert" class="provider-config-error">{{ configError }}</p>
          <ProviderConfigEditor :readonly="isReadOnly" v-model="form.configJson" :schema="filteredSchema" :capabilities="effectiveCapabilities" />
          <p v-if="!configError && !filteredSchema?.fields?.length" class="provider-local-note">{{ t('providers.studio.noParameters') }}</p>
        </section>

        <section v-if="hasDiagnostic" ref="diagnosticSection" tabindex="-1" class="provider-section provider-lab">
          <header class="provider-section-heading">
            <span class="section-number">04</span>
            <div><h3>{{ t('providers.studio.lab') }}</h3><p>{{ t('providers.studio.labHint') }}</p></div>
            <FlaskConical :size="18" aria-hidden="true" />
          </header>
          <ProviderTestPanel v-if="open" :type="form.type" :adapter="form.adapter" :model="typeof form.configJson.model === 'string' && form.configJson.model ? form.configJson.model : form.model" :capabilities="descriptor?.capabilities" :saved-key="isReadOnly ? provider?.id : undefined" :draft="isReadOnly ? undefined : { type: form.type as 'asr' | 'llm' | 'tts', adapter: form.adapter, config_json: form.configJson, ...(form.apiKey ? { api_key: form.apiKey } : provider && ['openai', 'chillaudio_ws'].includes(form.adapter) && form.adapter === provider.adapter && provider.desiredRevision ? { saved_credential: { key: provider.id, expected_revision: provider.desiredRevision } } : {}) }" />

        </section>
      </form>
    </div>
    <template #footer>
      <div class="provider-studio-footer">
        <p><LockKeyhole v-if="isReadOnly" :size="14" /><Check v-else :size="14" />{{ t(isReadOnly ? 'providers.studio.readOnly' : 'providers.studio.saveHint') }}</p>
        <div class="flex items-center gap-2">
          <Button type="button" variant="ghost" @click="open = false">{{ t('common.cancel') }}</Button>
          <Button type="submit" :form="formId" :disabled="isReadOnly">
            {{ provider ? t('common.saveChanges') : t('providers.create') }}<ArrowUpRight class="size-4" />
          </Button>
        </div>
      </div>
    </template>
  </BaseModal>
</template>

<style scoped src="./provider-form-modal.css"></style>

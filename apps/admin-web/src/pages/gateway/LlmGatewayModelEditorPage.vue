<script setup lang="ts">
import { ArrowLeft, Check, Info, Trash2 } from '@lucide/vue'
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { llmGatewayApi } from '@/api/llm-gateway'
import type {
  GatewayAccountView,
  GatewayPoolDetailView,
  GatewayPoolModelView,
  GatewayVendor,
  GatewayVendorInfo,
  GatewayVendorModelPreset,
} from '@/api/types/llm-gateway'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useErrorToast } from '@/composables/useNotify'
import { ALL_CAPABILITY_TAGS } from './llmGatewayUi'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()

const providerKeyParam = computed(() =>
  route.params.providerKey ? String(route.params.providerKey) : '',
)
const isCreateMode = computed(
  () => route.name === 'llm-gateway-model-create' || !providerKeyParam.value,
)

const vendors = ref<GatewayVendorInfo[]>([])
const accounts = ref<GatewayAccountView[]>([])
const poolDetailsByVendor = ref<Partial<Record<GatewayVendor, GatewayPoolDetailView>>>({})
const existingModel = ref<GatewayPoolModelView | null>(null)

const selectedVendor = ref<GatewayVendor>('openai_codex')
const modelId = ref('')
const displayName = ref('')
const gatewayBinding = ref('pool:openai_codex')
const baseUrl = ref('')
const tags = ref<string[]>(['chat'])
const reasoningEffort = ref('')
const maxTokens = ref<string | number>(8192)
const contextLimitTokens = ref<string | number>(128000)
const temperaturePercent = ref<string | number>('')
const timeoutMs = ref<number>(60000)
const enabled = ref(true)

const loading = ref(true)
const saving = ref(false)
const deleteOpen = ref(false)
const errorMessage = ref('')
useErrorToast(() => errorMessage.value, { retry: () => load() })

const vendorInfo = computed(
  () => vendors.value.find((v) => v.vendor === selectedVendor.value) ?? null,
)

const vendorAccounts = computed(() =>
  accounts.value.filter((a) => a.vendor === selectedVendor.value),
)

const currentPresets = computed<GatewayVendorModelPreset[]>(
  () => poolDetailsByVendor.value[selectedVendor.value]?.preset_models ?? [],
)

function applyPreset(preset: GatewayVendorModelPreset) {
  modelId.value = preset.model
  displayName.value = preset.display_name
  tags.value = preset.tags.length ? [...preset.tags] : ['chat']
  reasoningEffort.value = preset.default_reasoning_effort ?? ''
  maxTokens.value = preset.default_max_tokens != null ? String(preset.default_max_tokens) : ''
  contextLimitTokens.value =
    preset.default_context_limit_tokens != null
      ? String(preset.default_context_limit_tokens)
      : ''
  temperaturePercent.value =
    preset.default_temperature_percent != null
      ? String(preset.default_temperature_percent)
      : ''
  timeoutMs.value = preset.default_timeout_ms ?? 60000
}

function toggleTag(tag: string) {
  if (tags.value.includes(tag)) {
    if (tags.value.length > 1) {
      tags.value = tags.value.filter((item) => item !== tag)
    }
  } else {
    tags.value = [...tags.value, tag]
  }
}

async function ensurePoolDetail(vendor: GatewayVendor) {
  if (poolDetailsByVendor.value[vendor]) return poolDetailsByVendor.value[vendor]!
  try {
    const res = await llmGatewayApi.getPoolDetail(vendor)
    poolDetailsByVendor.value = {
      ...poolDetailsByVendor.value,
      [vendor]: res.pool,
    }
    return res.pool
  } catch {
    return null
  }
}

function populateFromExisting(m: GatewayPoolModelView) {
  existingModel.value = m
  selectedVendor.value = m.vendor
  modelId.value = m.model
  displayName.value = m.display_name
  gatewayBinding.value = m.gateway_binding
  baseUrl.value = m.base_url ?? ''
  tags.value = m.tags.length ? [...m.tags] : ['chat']
  reasoningEffort.value = m.reasoning_effort ?? ''
  maxTokens.value = m.max_tokens != null ? String(m.max_tokens) : ''
  contextLimitTokens.value =
    m.context_limit_tokens != null ? String(m.context_limit_tokens) : ''
  temperaturePercent.value =
    m.temperature_percent != null ? String(m.temperature_percent) : ''
  timeoutMs.value = m.timeout_ms || 60000
  enabled.value = m.enabled
}

async function load() {
  loading.value = true
  errorMessage.value = ''
  try {
    const [vendorRes, accountRes] = await Promise.all([
      llmGatewayApi.listVendors(),
      llmGatewayApi.listAccounts(),
    ])
    vendors.value = vendorRes.vendors
    accounts.value = accountRes.accounts

    const queryVendor = String(route.query.vendor || '') as GatewayVendor
    if (isCreateMode.value) {
      const initialVendor: GatewayVendor =
        vendorRes.vendors.find((v) => v.vendor === queryVendor)?.vendor ??
        vendorRes.vendors[0]?.vendor ??
        'openai_codex'
      selectedVendor.value = initialVendor
      gatewayBinding.value = `pool:${initialVendor}`
      const detail = await ensurePoolDetail(initialVendor)
      const presetQuery = String(route.query.preset || '').trim()
      if (presetQuery && detail) {
        const matchedPreset = detail.preset_models.find((p) => p.model === presetQuery)
        if (matchedPreset) {
          applyPreset(matchedPreset)
        } else {
          modelId.value = presetQuery
          displayName.value = presetQuery
        }
      }
    } else {
      let found: GatewayPoolModelView | null = null
      for (const v of vendorRes.vendors) {
        const match = (v.models ?? []).find(
          (m) => m.provider_key === providerKeyParam.value,
        )
        if (match) {
          found = match
          break
        }
      }
      if (!found && queryVendor) {
        const detail = await ensurePoolDetail(queryVendor)
        found =
          detail?.models.find((m) => m.provider_key === providerKeyParam.value) ?? null
      }
      if (found) {
        populateFromExisting(found)
        void ensurePoolDetail(found.vendor)
      }
    }
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    loading.value = false
  }
}

function onVendorChange() {
  if (!isCreateMode.value) return
  gatewayBinding.value = `pool:${selectedVendor.value}`
  void ensurePoolDetail(selectedVendor.value)
}

function parseOptionalInt(raw: string | number): number | null {
  const trimmed = String(raw ?? '').trim()
  if (!trimmed) return null
  const num = Number(trimmed)
  return Number.isFinite(num) && num >= 0 ? Math.round(num) : null
}

async function saveModel() {
  const trimmedModel = modelId.value.trim()
  if (!trimmedModel) {
    errorMessage.value = t('llmGateway.modelModal.modelRequired')
    return
  }
  if (!tags.value.length) {
    errorMessage.value = t('llmGateway.modelModal.tagsRequired')
    return
  }

  saving.value = true
  errorMessage.value = ''
  const payload = {
    model: trimmedModel,
    display_name: displayName.value.trim() || trimmedModel,
    gateway_binding: gatewayBinding.value.trim() || `pool:${selectedVendor.value}`,
    base_url: baseUrl.value.trim() || null,
    tags: tags.value,
    reasoning_effort: reasoningEffort.value.trim() || null,
    max_tokens: parseOptionalInt(maxTokens.value),
    context_limit_tokens: parseOptionalInt(contextLimitTokens.value),
    temperature_percent: parseOptionalInt(temperaturePercent.value),
    timeout_ms: Number(timeoutMs.value) || 60000,
    enabled: enabled.value,
  }

  try {
    if (isCreateMode.value) {
      await llmGatewayApi.createPoolModel(selectedVendor.value, payload)
    } else if (existingModel.value) {
      await llmGatewayApi.updatePoolModel(
        selectedVendor.value,
        existingModel.value.provider_key,
        payload,
      )
    }
    await router.push({
      name: 'llm-gateway-pool',
      params: { vendor: selectedVendor.value },
      query: { tab: 'models' },
    })
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

async function confirmDeleteModel() {
  if (!existingModel.value) return
  saving.value = true
  errorMessage.value = ''
  try {
    await llmGatewayApi.deletePoolModel(
      existingModel.value.vendor,
      existingModel.value.provider_key,
    )
    deleteOpen.value = false
    await router.push({ name: 'llm-gateway-models' })
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

function goBack() {
  if (isCreateMode.value && !route.query.vendor) {
    void router.push({ name: 'llm-gateway-models' })
  } else {
    void router.push({
      name: 'llm-gateway-pool',
      params: { vendor: selectedVendor.value },
    })
  }
}

watch(
  () => [route.name, route.params.providerKey, route.query.vendor, route.query.preset],
  () => {
    void load()
  },
)

onMounted(() => {
  void load()
})
</script>

<template>
  <section class="space-y-4" data-testid="llm-gateway-model-editor-page">
    <div class="flex flex-wrap items-center justify-between gap-2">
      <button
        type="button"
        data-detail-back
        class="inline-flex items-center gap-1.5 rounded-lg border border-border/80 bg-surface/60 px-2.5 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:border-studio-violet/40 hover:text-foreground"
        @click="goBack"
      >
        <ArrowLeft class="size-3.5" aria-hidden="true" />
        {{
          isCreateMode && !route.query.vendor
            ? t('llmGateway.backToModels')
            : t('llmGateway.backToPool')
        }}
      </button>
    </div>

    <header class="flex flex-wrap items-start justify-between gap-3">
      <div class="space-y-1">
        <p class="text-[11px] font-bold tracking-[0.14em] text-studio-violet uppercase">
          {{ t('llmGateway.modelPage.eyebrow') }}
        </p>
        <h1 class="text-xl font-bold tracking-tight sm:text-2xl">
          {{
            isCreateMode
              ? t('llmGateway.modelPage.createTitle')
              : t('llmGateway.modelPage.editTitle', {
                  name: displayName || modelId,
                })
          }}
        </h1>
        <p class="max-w-2xl text-xs text-muted-foreground sm:text-sm">
          {{ t('llmGateway.modelPage.subtitle') }}
        </p>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          data-action="save-model"
          :disabled="saving"
          @click="saveModel"
        >
          <Check class="size-4" aria-hidden="true" />
          {{ t('llmGateway.modelPage.saveBtn') }}
        </Button>
      </div>
    </header>

    <form
      data-testid="pool-model-form"
      class="grid gap-4 lg:grid-cols-[minmax(0,1fr)_315px]"
      @submit.prevent="saveModel"
    >
      <div class="space-y-3.5">
        <!-- Model Info Section -->
        <section class="studio-panel space-y-4 p-4">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div>
              <h2 class="text-sm font-bold text-foreground">
                {{ t('llmGateway.modelPage.infoTitle') }}
              </h2>
              <p class="text-xs text-muted-foreground">
                {{ t('llmGateway.modelPage.infoSubtitle') }}
              </p>
            </div>
            <Badge variant="outline" class="text-xs text-studio-violet">
              {{ vendorInfo?.display_name || selectedVendor }}
            </Badge>
          </div>

          <!-- Preset Chips -->
          <div v-if="currentPresets.length" class="space-y-1.5">
            <span class="block text-xs font-semibold text-muted-foreground">
              {{ t('llmGateway.modelModal.presetLabel') }}
            </span>
            <div class="flex flex-wrap gap-1.5">
              <button
                v-for="preset in currentPresets"
                :key="preset.model"
                type="button"
                :data-preset-model="preset.model"
                class="inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1 text-xs transition-colors"
                :class="
                  modelId === preset.model
                    ? 'border-studio-violet bg-studio-violet/10 font-semibold text-studio-violet'
                    : 'border-border/80 bg-surface text-muted-foreground hover:border-studio-violet/60 hover:text-foreground'
                "
                @click="applyPreset(preset)"
              >
                <span class="font-medium text-foreground">{{ preset.display_name }}</span>
                <code class="font-mono text-[11px] opacity-80">{{ preset.model }}</code>
              </button>
            </div>
          </div>

          <div class="grid gap-3 sm:grid-cols-2">
            <label class="block space-y-1">
              <span class="text-xs font-semibold">Vendor</span>
              <select
                v-model="selectedVendor"
                data-model-field="vendor"
                :disabled="!isCreateMode"
                class="admin-input text-xs"
                @change="onVendorChange"
              >
                <option v-for="v in vendors" :key="v.vendor" :value="v.vendor">
                  {{ v.display_name }}
                </option>
              </select>
            </label>

            <label class="block space-y-1">
              <span class="text-xs font-semibold">Model ID *</span>
              <input
                v-model="modelId"
                data-testid="pool-model-id-input"
                data-model-field="model"
                class="admin-input font-mono text-xs"
                :placeholder="t('llmGateway.modelModal.modelPlaceholder')"
                maxlength="256"
                required
              />
            </label>

            <label class="block space-y-1">
              <span class="text-xs font-semibold">Display name</span>
              <input
                v-model="displayName"
                data-testid="pool-model-name-input"
                data-model-field="name"
                class="admin-input text-xs"
                :placeholder="t('llmGateway.modelModal.displayNamePlaceholder')"
                maxlength="128"
              />
            </label>

            <label class="block space-y-1">
              <span class="text-xs font-semibold">Gateway Binding</span>
              <select
                v-model="gatewayBinding"
                data-model-field="binding"
                class="admin-input font-mono text-xs"
              >
                <option :value="`pool:${selectedVendor}`">pool:{{ selectedVendor }}</option>
                <option
                  v-for="acc in vendorAccounts"
                  :key="acc.account_key"
                  :value="`account:${acc.account_key}`"
                >
                  account:{{ acc.account_key }} ({{ acc.label }})
                </option>
              </select>
            </label>
          </div>

          <div class="h-px bg-border/60" />

          <div class="space-y-2">
            <h3 class="text-xs font-bold text-foreground">Capability Tags</h3>
            <div class="flex flex-wrap gap-1.5">
              <button
                v-for="tag in ALL_CAPABILITY_TAGS"
                :key="tag"
                type="button"
                :data-tag-option="tag"
                :data-tag="tag"
                class="rounded-lg border px-2.5 py-1 font-mono text-xs transition-colors"
                :class="
                  tags.includes(tag)
                    ? 'border-studio-violet bg-studio-violet/15 font-semibold text-studio-violet'
                    : 'border-border bg-surface text-muted-foreground hover:text-foreground'
                "
                @click="toggleTag(tag)"
              >
                {{ tag }}
              </button>
            </div>
          </div>
        </section>

        <!-- Execution Parameters Section -->
        <section class="studio-panel space-y-3.5 p-4">
          <h2 class="text-sm font-bold text-foreground">
            {{ t('llmGateway.modelPage.execParamsTitle') }}
          </h2>

          <div class="grid gap-3 sm:grid-cols-2">
            <label class="block space-y-1">
              <span class="text-xs font-semibold">Max Tokens</span>
              <input
                v-model="maxTokens"
                data-testid="pool-model-max-tokens-input"
                data-model-field="max"
                type="number"
                min="1"
                class="admin-input text-xs"
              />
            </label>

            <label class="block space-y-1">
              <span class="text-xs font-semibold">Context Limit Tokens</span>
              <input
                v-model="contextLimitTokens"
                data-model-field="context"
                type="number"
                min="0"
                class="admin-input text-xs"
              />
            </label>

            <label class="block space-y-1">
              <span class="text-xs font-semibold">Reasoning Effort</span>
              <select
                v-model="reasoningEffort"
                data-testid="pool-model-reasoning-select"
                data-model-field="reasoning"
                class="admin-input text-xs"
              >
                <option value="">None / Inherit</option>
                <option value="low">low</option>
                <option value="medium">medium</option>
                <option value="high">high</option>
              </select>
            </label>

            <label class="block space-y-1">
              <span class="text-xs font-semibold">Temperature (%)</span>
              <input
                v-model="temperaturePercent"
                data-testid="pool-model-temperature-input"
                data-model-field="temp"
                type="number"
                min="0"
                max="200"
                class="admin-input text-xs"
              />
            </label>

            <label class="block space-y-1">
              <span class="text-xs font-semibold">Timeout (ms)</span>
              <input
                v-model.number="timeoutMs"
                data-model-field="timeout"
                type="number"
                min="1000"
                class="admin-input text-xs"
              />
            </label>

            <label class="block space-y-1">
              <span class="text-xs font-semibold">{{ t('llmGateway.modelModal.baseUrl') }}</span>
              <input
                v-model="baseUrl"
                data-testid="pool-model-base-url-input"
                data-model-field="baseUrl"
                class="admin-input font-mono text-xs"
                :placeholder="t('llmGateway.modelModal.baseUrlPlaceholder')"
              />
            </label>
          </div>
        </section>
      </div>

      <aside class="space-y-3.5">
        <section class="studio-panel overflow-hidden">
          <div class="border-b border-border/70 px-4 py-3">
            <h2 class="text-sm font-bold text-foreground">
              {{ t('llmGateway.modelPage.statusCardTitle') }}
            </h2>
          </div>

          <div class="space-y-3 p-4">
            <div class="flex items-center justify-between gap-2 rounded-xl border border-border/70 bg-surface/40 p-3">
              <div>
                <div class="text-xs font-bold text-foreground">Enabled</div>
                <div class="text-[11px] text-muted-foreground">
                  {{ t('llmGateway.modelModal.enabled') }}
                </div>
              </div>
              <button
                type="button"
                role="switch"
                data-action="toggle-model-draft"
                :aria-checked="enabled"
                class="relative h-6 w-11 shrink-0 rounded-full border transition-colors"
                :class="
                  enabled
                    ? 'border-studio-violet bg-studio-violet'
                    : 'border-border bg-muted'
                "
                @click="enabled = !enabled"
              >
                <span
                  class="block size-4 rounded-full bg-white transition-transform"
                  :class="enabled ? 'translate-x-5.5' : 'translate-x-0.5'"
                />
              </button>
            </div>

            <dl class="divide-y divide-border/60 text-xs">
              <div class="flex items-center justify-between py-2">
                <dt class="text-muted-foreground">Vendor</dt>
                <dd class="text-foreground">{{ vendorInfo?.display_name || selectedVendor }}</dd>
              </div>
              <div class="flex items-center justify-between py-2">
                <dt class="text-muted-foreground">Binding</dt>
                <dd class="font-mono text-foreground">{{ gatewayBinding }}</dd>
              </div>
              <div class="flex items-center justify-between py-2">
                <dt class="text-muted-foreground">Template usage</dt>
                <dd class="text-foreground">{{ existingModel?.template_usage_count ?? 0 }}</dd>
              </div>
            </dl>

            <Button
              type="submit"
              size="sm"
              class="w-full"
              data-testid="pool-model-submit"
              :disabled="saving"
            >
              <Check class="size-4" aria-hidden="true" />
              {{ t('llmGateway.modelPage.saveBtn') }}
            </Button>

            <Button
              v-if="!isCreateMode && existingModel"
              type="button"
              variant="outline"
              size="sm"
              class="w-full border-danger/30 text-danger hover:bg-danger/10"
              data-action="ask-delete-model"
              :disabled="saving || existingModel.template_usage_count > 0"
              @click="deleteOpen = true"
            >
              <Trash2 class="size-4" aria-hidden="true" />
              {{ t('llmGateway.modelDelete.confirm') }}
            </Button>
          </div>
        </section>

        <div class="flex items-start gap-2.5 rounded-xl border border-studio-violet/25 bg-studio-violet/5 p-3.5 text-xs">
          <Info class="mt-0.5 size-4 shrink-0 text-studio-violet" aria-hidden="true" />
          <p class="text-muted-foreground">
            {{ t('llmGateway.modelModal.description') }}
          </p>
        </div>
      </aside>
    </form>

    <ConfirmDialog
      v-model="deleteOpen"
      :title="t('llmGateway.modelDelete.title')"
      :description="
        existingModel
          ? `${existingModel.display_name} (${existingModel.model} · ${existingModel.provider_key})`
          : ''
      "
      :confirm-label="t('llmGateway.modelDelete.confirm')"
      destructive
      @confirm="confirmDeleteModel"
    />
  </section>
</template>

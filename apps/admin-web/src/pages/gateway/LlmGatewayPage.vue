<script setup lang="ts">
import {
  AlertTriangle,
  Boxes,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Clock,
  Cpu,
  KeyRound,
  Pencil,
  Plus,
  RefreshCw,
  RotateCcw,
  ShieldAlert,
  Trash2,
} from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { RouterLink } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { llmGatewayApi } from '@/api/llm-gateway'
import type {
  GatewayAccountQuota,
  GatewayAccountStatus,
  GatewayAccountView,
  GatewayPoolDetailView,
  GatewayPoolModelView,
  GatewayQuotaWindow,
  GatewayVendor,
  GatewayVendorInfo,
  GatewayVendorModelPreset,
  OAuthLoginMode,
} from '@/api/types/llm-gateway'
import BaseModal from '@/components/admin/BaseModal.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

import OAuthLoginModal from './OAuthLoginModal.vue'

const ALL_CAPABILITY_TAGS = [
  'chat',
  'reasoning',
  'vision',
  'image',
  'video',
  'audio',
  'embedding',
] as const

interface VendorPoolGroup {
  vendor: GatewayVendor
  displayName: string
  poolBinding: string
  loginMode: OAuthLoginMode
  accounts: GatewayAccountView[]
  models: GatewayPoolModelView[]
  activeCount: number
  cooldownCount: number
}

const { t } = useI18n()

const vendors = ref<GatewayVendorInfo[]>([])
const accounts = ref<GatewayAccountView[]>([])
const quotasByAccount = ref<Record<string, GatewayAccountQuota>>({})
const selectedVendorFilter = ref<GatewayVendor | 'all'>('all')
const collapsedPools = ref<Record<string, boolean>>({})
const loading = ref(false)
const loadingQuotas = ref(false)
const quotaBusyKey = ref('')
const busyKey = ref('')
const errorMessage = ref('')

const loginModalOpen = ref(false)
const loginInitialVendor = ref<GatewayVendor | undefined>(undefined)

const editingAccount = ref<GatewayAccountView | null>(null)
const editOpen = ref(false)
const editLabel = ref('')
const editPriority = ref(100)

const deletingAccount = ref<GatewayAccountView | null>(null)
const deleteOpen = ref(false)

// Pool Detail & Pool Model CRUD state
const poolDetailOpen = ref(false)
const poolDetailLoading = ref(false)
const activePoolDetail = ref<GatewayPoolDetailView | null>(null)
const presetCacheByVendor = ref<Record<string, GatewayVendorModelPreset[]>>({})

const modelModalOpen = ref(false)
const modelModalMode = ref<'create' | 'edit'>('create')
const modelModalVendor = ref<GatewayVendor>('anthropic')
const editingPoolModel = ref<GatewayPoolModelView | null>(null)
const modelForm = reactive({
  modelId: '',
  displayName: '',
  gatewayBinding: '',
  baseUrl: '',
  tags: ['chat'] as string[],
  reasoningEffort: '',
  maxTokens: '',
  contextLimitTokens: '',
  temperaturePercent: '',
  timeoutMs: 60000,
  enabled: true,
})

const deletingPoolModel = ref<GatewayPoolModelView | null>(null)
const deleteModelOpen = ref(false)

let loadController: AbortController | undefined
let cooldownTicker: ReturnType<typeof setInterval> | undefined

const activeAccountsCount = computed(
  () => accounts.value.filter((item) => item.status === 'active').length,
)

const coolingDownAccountsCount = computed(
  () => accounts.value.filter((item) => item.status === 'cooling_down').length,
)

const allPoolGroups = computed<VendorPoolGroup[]>(() => {
  const knownVendors = new Set<GatewayVendor>(vendors.value.map((v) => v.vendor))
  for (const acc of accounts.value) {
    knownVendors.add(acc.vendor)
  }

  const groups: VendorPoolGroup[] = []
  for (const vendor of knownVendors) {
    const info = vendors.value.find((v) => v.vendor === vendor)
    const vendorAccounts = accounts.value.filter((acc) => acc.vendor === vendor)
    const vendorModels = info?.models ?? []
    groups.push({
      vendor,
      displayName: info?.display_name ?? vendor.replace(/_/g, ' '),
      poolBinding: info?.pool_binding ?? `pool:${vendor}`,
      loginMode: info?.login_mode ?? 'pkce',
      accounts: vendorAccounts,
      models: vendorModels,
      activeCount: vendorAccounts.filter((acc) => acc.status === 'active').length,
      cooldownCount: vendorAccounts.filter((acc) => acc.status === 'cooling_down').length,
    })
  }
  return groups
})

const populatedPoolGroups = computed(() =>
  allPoolGroups.value.filter((group) => group.accounts.length > 0 || group.models.length > 0),
)

const visiblePoolGroups = computed(() => {
  if (selectedVendorFilter.value === 'all') {
    return populatedPoolGroups.value
  }
  return populatedPoolGroups.value.filter((group) => group.vendor === selectedVendorFilter.value)
})

const emptyVendorPools = computed(() =>
  allPoolGroups.value.filter((group) => group.accounts.length === 0 && group.models.length === 0),
)

const areAllVisiblePoolsCollapsed = computed(
  () =>
    visiblePoolGroups.value.length > 0 &&
    visiblePoolGroups.value.every((group) => Boolean(collapsedPools.value[group.vendor])),
)

function isPoolCollapsed(vendor: GatewayVendor): boolean {
  return Boolean(collapsedPools.value[vendor])
}

function togglePoolCollapse(vendor: GatewayVendor) {
  collapsedPools.value = {
    ...collapsedPools.value,
    [vendor]: !collapsedPools.value[vendor],
  }
}

function toggleAllPools() {
  const shouldCollapse = !areAllVisiblePoolsCollapsed.value
  const next = { ...collapsedPools.value }
  for (const group of visiblePoolGroups.value) {
    next[group.vendor] = shouldCollapse
  }
  collapsedPools.value = next
}

function statusBadgeClass(status: GatewayAccountStatus): string {
  switch (status) {
    case 'active':
      return 'border-success/30 bg-success/10 text-success'
    case 'cooling_down':
      return 'border-warning/30 bg-warning/10 text-warning'
    case 'reauth_required':
      return 'border-danger/30 bg-danger/10 text-danger'
    case 'disabled':
    default:
      return 'border-border bg-muted/50 text-muted-foreground'
  }
}

function statusLabel(acc: GatewayAccountView): string {
  switch (acc.status) {
    case 'active':
      return t('llmGateway.status.active')
    case 'cooling_down':
      return acc.cooldown_remaining_secs > 0
        ? `${t('llmGateway.status.coolingDown')} (${acc.cooldown_remaining_secs}s)`
        : t('llmGateway.status.coolingDown')
    case 'reauth_required':
      return t('llmGateway.status.reauthRequired')
    case 'disabled':
    default:
      return t('llmGateway.status.disabled')
  }
}

function maskedCredentialSummary(acc: GatewayAccountView): string {
  const meta = acc.credential_metadata as Record<string, unknown> | null | undefined
  if (!meta || typeof meta !== 'object') return ''
  const masked = typeof meta.masked_key === 'string' ? meta.masked_key : ''
  const kv = typeof meta.key_version === 'number' ? `v${meta.key_version}` : ''
  return [masked, kv].filter(Boolean).join(' · ')
}

function formatTimestamp(epochSecs: number | null): string {
  if (!epochSecs) return '—'
  try {
    return new Date(epochSecs * 1000).toLocaleString()
  } catch {
    return '—'
  }
}

function hasDirectAccountBinding(acc: GatewayAccountView): boolean {
  return acc.bound_providers.some((b) => b.binding_mode === 'account')
}

function accountQuota(acc: GatewayAccountView): GatewayAccountQuota | null {
  return quotasByAccount.value[acc.account_key] ?? acc.quota ?? null
}

function quotaBarColorClass(pct: number, unlimited: boolean): string {
  if (unlimited || pct >= 70) return 'bg-emerald-500'
  if (pct >= 30) return 'bg-amber-500'
  return 'bg-rose-500'
}

function quotaTextColorClass(pct: number, unlimited: boolean): string {
  if (unlimited || pct >= 70) return 'text-emerald-600 dark:text-emerald-400'
  if (pct >= 30) return 'text-amber-600 dark:text-amber-400'
  return 'text-rose-600 dark:text-rose-400'
}

function formatResetCountdown(resetAt?: number | null): string {
  if (!resetAt) return ''
  const nowSecs = Math.floor(Date.now() / 1000)
  const diffSecs = resetAt - nowSecs
  if (diffSecs <= 0) return ''
  const days = Math.floor(diffSecs / 86400)
  const hours = Math.floor((diffSecs % 86400) / 3600)
  const mins = Math.max(1, Math.floor((diffSecs % 3600) / 60))
  if (days > 0) return `${days}d ${hours}h`
  if (hours > 0) return `${hours}h ${mins}m`
  return `${mins}m`
}

function formatWindowUsage(w: GatewayQuotaWindow): string {
  if (w.unlimited) return t('llmGateway.quota.unlimited')
  const pct = Math.round(w.remaining_percentage)
  if (w.unit === '%') return `${pct}%`
  const remaining = Math.max(0, Math.round(w.total - w.used))
  return `${remaining}/${Math.round(w.total)} (${pct}%)`
}

async function fetchAllQuotas(force = false) {
  if (!accounts.value.length || loadingQuotas.value) return
  loadingQuotas.value = true
  try {
    const vendorParam =
      selectedVendorFilter.value === 'all' ? undefined : selectedVendorFilter.value
    const res = await llmGatewayApi.listQuotas({ vendor: vendorParam, force })
    const next = { ...quotasByAccount.value }
    for (const q of res.quotas) {
      next[q.account_key] = q
    }
    quotasByAccount.value = next
  } catch (cause) {
    if (force) {
      errorMessage.value = formatApiError(cause)
    }
  } finally {
    loadingQuotas.value = false
  }
}

async function refreshSingleQuota(acc: GatewayAccountView) {
  if (quotaBusyKey.value) return
  quotaBusyKey.value = acc.account_key
  try {
    const res = await llmGatewayApi.getAccountQuota(acc.account_key, true)
    quotasByAccount.value = {
      ...quotasByAccount.value,
      [acc.account_key]: res.quota,
    }
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    quotaBusyKey.value = ''
  }
}

async function load() {
  loadController?.abort()
  const controller = new AbortController()
  loadController = controller
  loading.value = true
  errorMessage.value = ''
  try {
    const [vendorRes, accountRes] = await Promise.all([
      llmGatewayApi.listVendors(controller.signal),
      llmGatewayApi.listAccounts(undefined, controller.signal),
    ])
    if (controller.signal.aborted) return
    vendors.value = vendorRes.vendors
    accounts.value = accountRes.accounts
    const seeded: Record<string, GatewayAccountQuota> = { ...quotasByAccount.value }
    for (const acc of accountRes.accounts) {
      if (acc.quota) {
        seeded[acc.account_key] = acc.quota
      }
    }
    quotasByAccount.value = seeded
    if (accountRes.accounts.length > 0) {
      void fetchAllQuotas(false)
    }
  } catch (cause) {
    if (!controller.signal.aborted) {
      errorMessage.value = formatApiError(cause)
    }
  } finally {
    if (loadController === controller) {
      loading.value = false
    }
  }
}

function openLoginModal(vendor?: GatewayVendor) {
  loginInitialVendor.value = vendor
  loginModalOpen.value = true
}

function onOAuthCompleted() {
  void load()
}

async function refreshSingleAccount(acc: GatewayAccountView) {
  if (busyKey.value) return
  busyKey.value = acc.account_key
  errorMessage.value = ''
  try {
    await llmGatewayApi.refreshAccount(acc.account_key)
    await load()
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    busyKey.value = ''
  }
}

async function clearAccountCooldown(acc: GatewayAccountView) {
  if (busyKey.value) return
  busyKey.value = acc.account_key
  errorMessage.value = ''
  try {
    await llmGatewayApi.clearCooldown(acc.account_key)
    await load()
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    busyKey.value = ''
  }
}

async function toggleAccountEnabled(acc: GatewayAccountView) {
  if (busyKey.value) return
  busyKey.value = acc.account_key
  errorMessage.value = ''
  try {
    await llmGatewayApi.updateAccount(acc.account_key, { enabled: !acc.enabled })
    await load()
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    busyKey.value = ''
  }
}

function openEditModal(acc: GatewayAccountView) {
  editingAccount.value = acc
  editLabel.value = acc.label
  editPriority.value = acc.priority
  editOpen.value = true
}

async function saveAccountEdit() {
  const acc = editingAccount.value
  if (!acc || busyKey.value) return
  const trimmedLabel = editLabel.value.trim()
  if (!trimmedLabel) {
    errorMessage.value = t('llmGateway.editModal.labelRequired')
    return
  }
  const parsedPriority = Number(editPriority.value)
  const priority = Number.isFinite(parsedPriority) ? Math.trunc(parsedPriority) : 100
  busyKey.value = acc.account_key
  errorMessage.value = ''
  try {
    await llmGatewayApi.updateAccount(acc.account_key, {
      label: trimmedLabel,
      priority,
    })
    editOpen.value = false
    editingAccount.value = null
    await load()
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    busyKey.value = ''
  }
}

function promptDeleteAccount(acc: GatewayAccountView) {
  deletingAccount.value = acc
  deleteOpen.value = true
}

async function confirmDeleteAccount() {
  const acc = deletingAccount.value
  if (!acc || busyKey.value) return
  busyKey.value = acc.account_key
  errorMessage.value = ''
  try {
    await llmGatewayApi.deleteAccount(acc.account_key)
    deleteOpen.value = false
    deletingAccount.value = null
    await load()
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    busyKey.value = ''
  }
}

const currentModalPresets = computed<GatewayVendorModelPreset[]>(() => {
  if (activePoolDetail.value && activePoolDetail.value.vendor === modelModalVendor.value) {
    return activePoolDetail.value.preset_models
  }
  return presetCacheByVendor.value[modelModalVendor.value] ?? []
})

const currentVendorAccounts = computed<GatewayAccountView[]>(() =>
  accounts.value.filter((acc) => acc.vendor === modelModalVendor.value),
)

function vendorDisplayName(vendor: GatewayVendor): string {
  return (
    vendors.value.find((v) => v.vendor === vendor)?.display_name ??
    vendor.replace(/_/g, ' ')
  )
}

async function fetchPoolDetailForVendor(vendor: GatewayVendor) {
  poolDetailLoading.value = true
  try {
    const res = await llmGatewayApi.getPoolDetail(vendor)
    activePoolDetail.value = res.pool
    presetCacheByVendor.value = {
      ...presetCacheByVendor.value,
      [vendor]: res.pool.preset_models,
    }
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    poolDetailLoading.value = false
  }
}

async function openPoolDetailModal(vendor: GatewayVendor) {
  errorMessage.value = ''
  poolDetailOpen.value = true
  await fetchPoolDetailForVendor(vendor)
}

async function ensurePresetsLoaded(vendor: GatewayVendor) {
  if (presetCacheByVendor.value[vendor]?.length) return
  try {
    const res = await llmGatewayApi.getPoolDetail(vendor)
    presetCacheByVendor.value = {
      ...presetCacheByVendor.value,
      [vendor]: res.pool.preset_models,
    }
  } catch {
    // Non-fatal; user can still enter custom model ID
  }
}

function applyPresetToForm(preset: GatewayVendorModelPreset) {
  modelForm.modelId = preset.model
  if (modelModalMode.value === 'create' || !modelForm.displayName.trim()) {
    modelForm.displayName = preset.display_name
  }
  modelForm.tags = [...preset.tags]
  modelForm.maxTokens =
    preset.default_max_tokens != null ? String(preset.default_max_tokens) : ''
  modelForm.contextLimitTokens =
    preset.default_context_limit_tokens != null
      ? String(preset.default_context_limit_tokens)
      : ''
  if (!preset.supports_reasoning_effort) {
    modelForm.reasoningEffort = ''
  }
}

function openCreatePoolModelModal(vendor: GatewayVendor, preset?: GatewayVendorModelPreset) {
  errorMessage.value = ''
  modelModalMode.value = 'create'
  modelModalVendor.value = vendor
  editingPoolModel.value = null
  modelForm.gatewayBinding = `pool:${vendor}`
  modelForm.baseUrl = ''
  modelForm.reasoningEffort = ''
  modelForm.temperaturePercent = ''
  modelForm.timeoutMs = 60000
  modelForm.enabled = true
  if (preset) {
    applyPresetToForm(preset)
  } else {
    modelForm.modelId = ''
    modelForm.displayName = ''
    modelForm.tags = ['chat']
    modelForm.maxTokens = ''
    modelForm.contextLimitTokens = ''
  }
  modelModalOpen.value = true
  void ensurePresetsLoaded(vendor)
}

function openEditPoolModelModal(model: GatewayPoolModelView) {
  errorMessage.value = ''
  modelModalMode.value = 'edit'
  modelModalVendor.value = model.vendor
  editingPoolModel.value = model
  modelForm.modelId = model.model
  modelForm.displayName = model.display_name
  modelForm.gatewayBinding = model.gateway_binding
  modelForm.baseUrl = model.base_url ?? ''
  modelForm.tags = model.tags.length ? [...model.tags] : ['chat']
  modelForm.reasoningEffort = model.reasoning_effort ?? ''
  modelForm.maxTokens = model.max_tokens != null ? String(model.max_tokens) : ''
  modelForm.contextLimitTokens =
    model.context_limit_tokens != null ? String(model.context_limit_tokens) : ''
  modelForm.temperaturePercent =
    model.temperature_percent != null ? String(model.temperature_percent) : ''
  modelForm.timeoutMs = model.timeout_ms || 60000
  modelForm.enabled = model.enabled
  modelModalOpen.value = true
  void ensurePresetsLoaded(model.vendor)
}

function toggleFormTag(tag: string) {
  if (modelForm.tags.includes(tag)) {
    if (modelForm.tags.length > 1) {
      modelForm.tags = modelForm.tags.filter((tItem) => tItem !== tag)
    }
  } else {
    modelForm.tags = [...modelForm.tags, tag]
  }
}

function parseOptionalPositiveInt(raw: string): number | null {
  const trimmed = String(raw ?? '').trim()
  if (!trimmed) return null
  const num = Number(trimmed)
  return Number.isFinite(num) && num >= 0 ? Math.trunc(num) : null
}

async function savePoolModel() {
  if (busyKey.value) return
  const trimmedModel = modelForm.modelId.trim()
  if (!trimmedModel) {
    errorMessage.value = t('llmGateway.modelModal.modelRequired')
    return
  }
  if (!modelForm.tags.length) {
    errorMessage.value = t('llmGateway.modelModal.tagsRequired')
    return
  }
  const vendor = modelModalVendor.value
  const trimmedName = modelForm.displayName.trim()
  const trimmedBaseUrl = modelForm.baseUrl.trim()
  const maxTokens = parseOptionalPositiveInt(modelForm.maxTokens)
  const contextLimitTokens = parseOptionalPositiveInt(modelForm.contextLimitTokens)
  const temperaturePercent = parseOptionalPositiveInt(modelForm.temperaturePercent)
  const timeoutMs = Number.isFinite(Number(modelForm.timeoutMs))
    ? Math.max(1000, Math.trunc(Number(modelForm.timeoutMs)))
    : 60000
  const reasoningEffort = modelForm.reasoningEffort.trim() || null

  busyKey.value = editingPoolModel.value?.provider_key ?? `create:${vendor}`
  errorMessage.value = ''
  try {
    if (modelModalMode.value === 'create') {
      await llmGatewayApi.createPoolModel(vendor, {
        model: trimmedModel,
        display_name: trimmedName || undefined,
        gateway_binding: modelForm.gatewayBinding || `pool:${vendor}`,
        base_url: trimmedBaseUrl || undefined,
        tags: modelForm.tags,
        reasoning_effort: reasoningEffort,
        max_tokens: maxTokens,
        context_limit_tokens: contextLimitTokens,
        temperature_percent: temperaturePercent,
        timeout_ms: timeoutMs,
        enabled: modelForm.enabled,
      })
    } else if (editingPoolModel.value) {
      await llmGatewayApi.updatePoolModel(vendor, editingPoolModel.value.provider_key, {
        model: trimmedModel,
        display_name: trimmedName || editingPoolModel.value.display_name,
        gateway_binding: modelForm.gatewayBinding || `pool:${vendor}`,
        base_url: trimmedBaseUrl || null,
        tags: modelForm.tags,
        reasoning_effort: reasoningEffort,
        max_tokens: maxTokens,
        context_limit_tokens: contextLimitTokens,
        temperature_percent: temperaturePercent,
        timeout_ms: timeoutMs,
        enabled: modelForm.enabled,
      })
    }
    modelModalOpen.value = false
    editingPoolModel.value = null
    await load()
    if (poolDetailOpen.value && activePoolDetail.value?.vendor === vendor) {
      await fetchPoolDetailForVendor(vendor)
    }
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    busyKey.value = ''
  }
}

async function togglePoolModelEnabled(model: GatewayPoolModelView) {
  if (busyKey.value) return
  busyKey.value = model.provider_key
  errorMessage.value = ''
  try {
    await llmGatewayApi.updatePoolModel(model.vendor, model.provider_key, {
      enabled: !model.enabled,
    })
    await load()
    if (poolDetailOpen.value && activePoolDetail.value?.vendor === model.vendor) {
      await fetchPoolDetailForVendor(model.vendor)
    }
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    busyKey.value = ''
  }
}

function promptDeletePoolModel(model: GatewayPoolModelView) {
  deletingPoolModel.value = model
  deleteModelOpen.value = true
}

async function confirmDeletePoolModel() {
  const model = deletingPoolModel.value
  if (!model || busyKey.value) return
  busyKey.value = model.provider_key
  errorMessage.value = ''
  try {
    await llmGatewayApi.deletePoolModel(model.vendor, model.provider_key)
    deleteModelOpen.value = false
    deletingPoolModel.value = null
    await load()
    if (poolDetailOpen.value && activePoolDetail.value?.vendor === model.vendor) {
      await fetchPoolDetailForVendor(model.vendor)
    }
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    busyKey.value = ''
  }
}

onMounted(() => {
  cooldownTicker = setInterval(() => {
    let anyExpired = false
    for (const acc of accounts.value) {
      if (acc.status === 'cooling_down' && acc.cooldown_remaining_secs > 0) {
        acc.cooldown_remaining_secs = Math.max(0, acc.cooldown_remaining_secs - 1)
        if (acc.cooldown_remaining_secs === 0) {
          anyExpired = true
        }
      }
    }
    if (anyExpired && !loading.value) {
      void load()
    }
  }, 1000)
})

onBeforeUnmount(() => {
  loadController?.abort()
  if (cooldownTicker) {
    clearInterval(cooldownTicker)
    cooldownTicker = undefined
  }
})

void load()
</script>

<template>
  <main class="mx-auto max-w-7xl space-y-4">
    <!-- Compact Header -->
    <header class="flex flex-wrap items-center justify-between gap-3">
      <div class="flex items-start gap-3">
        <span
          class="flex size-9 shrink-0 items-center justify-center rounded-xl border border-studio-violet/20 bg-studio-violet/10 text-studio-violet"
        >
          <KeyRound class="size-4" aria-hidden="true" />
        </span>
        <div>
          <div class="flex flex-wrap items-center gap-2">
            <h1 class="text-lg font-semibold tracking-tight">
              {{ t('llmGateway.title') }}
            </h1>
            <span
              v-if="accounts.length"
              class="inline-flex items-center gap-1.5 rounded-full border border-border bg-surface px-2.5 py-0.5 text-xs font-medium text-muted-foreground"
            >
              <span class="size-2 rounded-full bg-success" aria-hidden="true" />
              {{ activeAccountsCount }}/{{ accounts.length }} {{ t('llmGateway.active') }}
            </span>
            <span
              v-if="coolingDownAccountsCount > 0"
              class="inline-flex items-center gap-1.5 rounded-full border border-warning/30 bg-warning/10 px-2.5 py-0.5 text-xs font-medium text-warning"
            >
              {{ coolingDownAccountsCount }} {{ t('llmGateway.cooldown') }}
            </span>
          </div>
          <p class="mt-0.5 text-xs text-muted-foreground">
            {{ t('llmGateway.subtitle') }}
          </p>
        </div>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <Button
          v-if="visiblePoolGroups.length > 0"
          variant="outline"
          size="sm"
          data-testid="toggle-all-pools"
          @click="toggleAllPools"
        >
          <ChevronRight
            v-if="areAllVisiblePoolsCollapsed"
            class="size-4"
            aria-hidden="true"
          />
          <ChevronDown v-else class="size-4" aria-hidden="true" />
          {{
            areAllVisiblePoolsCollapsed
              ? t('llmGateway.expandAllPools')
              : t('llmGateway.collapseAllPools')
          }}
        </Button>
        <Button
          v-if="accounts.length"
          variant="outline"
          size="sm"
          :disabled="loadingQuotas || loading"
          @click="fetchAllQuotas(true)"
        >
          <RefreshCw class="size-4" :class="{ 'animate-spin': loadingQuotas }" aria-hidden="true" />
          {{ t('llmGateway.refreshQuotas') }}
        </Button>
        <Button variant="outline" size="sm" :disabled="loading" @click="load">
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" aria-hidden="true" />
          {{ t('common.refresh') }}
        </Button>
        <Button size="sm" @click="openLoginModal()">
          <Plus class="size-4" aria-hidden="true" />
          {{ t('llmGateway.loginCta') }}
        </Button>
      </div>
    </header>

    <p
      v-if="errorMessage"
      role="alert"
      class="rounded-lg border border-danger/30 bg-danger/5 px-4 py-3 text-sm text-danger"
    >
      {{ errorMessage }}
    </p>

    <!-- Compact Vendor Filter Bar (only when accounts exist) -->
    <div
      v-if="populatedPoolGroups.length > 1"
      class="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-border/70 bg-surface/40 px-3 py-2"
    >
      <div class="flex flex-wrap items-center gap-1.5">
        <button
          type="button"
          class="rounded-lg px-2.5 py-1 text-xs font-medium whitespace-nowrap transition-colors"
          :class="
            selectedVendorFilter === 'all'
              ? 'bg-studio-violet text-white'
              : 'text-muted-foreground hover:bg-accent hover:text-foreground'
          "
          @click="selectedVendorFilter = 'all'"
        >
          {{ t('llmGateway.allAccounts') }} ({{ accounts.length }})
        </button>
        <button
          v-for="group in populatedPoolGroups"
          :key="group.vendor"
          type="button"
          class="inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium whitespace-nowrap transition-colors"
          :class="
            selectedVendorFilter === group.vendor
              ? 'bg-studio-violet text-white'
              : 'text-muted-foreground hover:bg-accent hover:text-foreground'
          "
          @click="
            selectedVendorFilter = selectedVendorFilter === group.vendor ? 'all' : group.vendor
          "
        >
          <span>{{ group.displayName }}</span>
          <span class="opacity-80">({{ group.accounts.length }})</span>
        </button>
      </div>
    </div>

    <!-- Loading / Empty States -->
    <div
      v-if="loading && !visiblePoolGroups.length"
      class="studio-panel p-10 text-center text-sm text-muted-foreground"
    >
      {{ t('llmGateway.loading') }}
    </div>

    <div
      v-else-if="!visiblePoolGroups.length"
      class="studio-panel space-y-3 p-10 text-center text-sm text-muted-foreground"
    >
      <p>{{ t('llmGateway.empty') }}</p>
      <Button
        size="sm"
        variant="outline"
        @click="openLoginModal(selectedVendorFilter === 'all' ? undefined : selectedVendorFilter)"
      >
        <Plus class="size-4" aria-hidden="true" />
        {{ t('llmGateway.emptyCta') }}
      </Button>
    </div>

    <!-- Grouped Vendor Pool Sections -->
    <div v-else class="space-y-3">
      <section
        v-for="group in visiblePoolGroups"
        :key="group.vendor"
        :data-pool-group="group.vendor"
        class="studio-panel overflow-hidden"
      >
        <!-- Pool Group Header with Expand / Collapse Action -->
        <div
          class="flex flex-wrap items-center justify-between gap-2 bg-surface/40 px-4 py-2.5"
          :class="{ 'border-b border-border/70': !isPoolCollapsed(group.vendor) }"
        >
          <button
            type="button"
            :aria-expanded="!isPoolCollapsed(group.vendor)"
            :data-pool-toggle="group.vendor"
            class="flex min-w-0 flex-wrap items-center gap-2 text-left transition-opacity hover:opacity-90"
            @click="togglePoolCollapse(group.vendor)"
          >
            <ChevronRight
              v-if="isPoolCollapsed(group.vendor)"
              class="size-4 shrink-0 text-muted-foreground"
              aria-hidden="true"
            />
            <ChevronDown
              v-else
              class="size-4 shrink-0 text-muted-foreground"
              aria-hidden="true"
            />
            <h2 class="text-sm font-semibold text-foreground">{{ group.displayName }}</h2>
            <code
              class="rounded border border-border/70 bg-background/70 px-2 py-0.5 font-mono text-xs text-muted-foreground"
            >
              {{ group.poolBinding }}
            </code>
            <Badge variant="outline" class="font-mono text-[10px]">
              {{ group.loginMode === 'device_code' ? 'Device Code' : 'PKCE' }}
            </Badge>
            <span
              class="inline-flex items-center gap-1 rounded-full border border-success/30 bg-success/10 px-2 py-0.5 text-xs font-medium text-success"
            >
              {{ group.activeCount }} {{ t('llmGateway.active') }}
            </span>
            <span
              v-if="group.cooldownCount > 0"
              class="inline-flex items-center gap-1 rounded-full border border-warning/30 bg-warning/10 px-2 py-0.5 text-xs font-medium text-warning"
            >
              {{ group.cooldownCount }} {{ t('llmGateway.cooldown') }}
            </span>
            <span class="text-xs text-muted-foreground">
              ({{ t('llmGateway.total') }}: {{ group.accounts.length }})
            </span>
            <span
              class="inline-flex items-center gap-1 rounded-full border border-studio-violet/30 bg-studio-violet/10 px-2 py-0.5 text-xs font-medium text-studio-violet"
              :data-pool-model-count="group.vendor"
            >
              <Cpu class="size-3" aria-hidden="true" />
              {{ t('llmGateway.pool.modelsCount', { count: group.models.length }) }}
            </span>
          </button>

          <div class="flex shrink-0 flex-wrap items-center gap-1.5">
            <Button
              variant="outline"
              size="sm"
              class="h-7 px-2.5 text-xs whitespace-nowrap"
              :data-pool-detail="group.vendor"
              @click="openPoolDetailModal(group.vendor)"
            >
              <Boxes class="size-3.5" aria-hidden="true" />
              {{ t('llmGateway.pool.detailBtn') }}
            </Button>
            <Button
              variant="outline"
              size="sm"
              class="h-7 px-2.5 text-xs whitespace-nowrap"
              :data-pool-add-model="group.vendor"
              @click="openCreatePoolModelModal(group.vendor)"
            >
              <Plus class="size-3.5" aria-hidden="true" />
              {{ t('llmGateway.pool.addModelBtn') }}
            </Button>
            <Button
              variant="outline"
              size="sm"
              class="h-7 px-2.5 text-xs whitespace-nowrap"
              @click="openLoginModal(group.vendor)"
            >
              <Plus class="size-3.5" aria-hidden="true" />
              {{ t('llmGateway.addToPool') }}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              class="h-7 px-2 text-xs text-muted-foreground whitespace-nowrap"
              :aria-expanded="!isPoolCollapsed(group.vendor)"
              @click="togglePoolCollapse(group.vendor)"
            >
              <ChevronRight
                v-if="isPoolCollapsed(group.vendor)"
                class="size-3.5"
                aria-hidden="true"
              />
              <ChevronDown v-else class="size-3.5" aria-hidden="true" />
              {{
                isPoolCollapsed(group.vendor)
                  ? t('llmGateway.expandPool')
                  : t('llmGateway.collapsePool')
              }}
            </Button>
          </div>
        </div>

        <!-- Collapsible Pool Account Table & Pool Models Section -->
        <div v-show="!isPoolCollapsed(group.vendor)">
          <div v-if="group.accounts.length" class="overflow-x-auto">
          <table class="w-full text-left text-sm">
            <thead class="border-b border-border/60 bg-surface/30 text-xs uppercase text-muted-foreground">
              <tr>
                <th class="px-4 py-2.5 font-medium">{{ t('llmGateway.col.account') }}</th>
                <th class="px-4 py-2.5 font-medium">{{ t('llmGateway.col.status') }}</th>
                <th class="px-4 py-2.5 font-medium">{{ t('llmGateway.col.quota') }}</th>
                <th class="px-4 py-2.5 font-medium">{{ t('llmGateway.col.boundProviders') }}</th>
                <th class="px-4 py-2.5 text-right font-medium">{{ t('llmGateway.col.actions') }}</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-border/70">
              <tr
                v-for="acc in group.accounts"
                :key="acc.account_key"
                class="hover:bg-accent/20"
              >
                <!-- Account Label, Priority, Email & Credential Metadata -->
                <td class="px-4 py-2.5 align-middle">
                  <div class="flex flex-wrap items-center gap-2">
                    <span class="font-medium text-foreground">{{ acc.label }}</span>
                    <span
                      class="rounded border border-border/80 bg-surface px-1.5 py-0.5 font-mono text-[11px] tabular-nums text-muted-foreground"
                      :title="t('llmGateway.col.priority')"
                    >
                      P:{{ acc.priority }}
                    </span>
                  </div>
                  <div v-if="acc.account_email" class="text-xs text-muted-foreground">
                    {{ acc.account_email }}
                  </div>
                  <div class="mt-0.5 flex flex-wrap items-center gap-1.5 font-mono text-[11px] text-muted-foreground">
                    <code class="whitespace-nowrap">account:{{ acc.account_key }}</code>
                    <span v-if="maskedCredentialSummary(acc)" class="whitespace-nowrap">
                      · {{ maskedCredentialSummary(acc) }}
                    </span>
                  </div>
                </td>

                <!-- Status & Expiry -->
                <td class="px-4 py-2.5 align-middle whitespace-nowrap">
                  <span
                    class="inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium whitespace-nowrap"
                    :class="statusBadgeClass(acc.status)"
                  >
                    <CheckCircle2 v-if="acc.status === 'active'" class="size-3.5" aria-hidden="true" />
                    <Clock v-else-if="acc.status === 'cooling_down'" class="size-3.5" aria-hidden="true" />
                    <ShieldAlert
                      v-else-if="acc.status === 'reauth_required'"
                      class="size-3.5"
                      aria-hidden="true"
                    />
                    {{ statusLabel(acc) }}
                  </span>
                  <div class="mt-1 text-[11px] text-muted-foreground">
                    {{ t('llmGateway.expiresAt') }}: {{ formatTimestamp(acc.expires_at) }}
                  </div>
                  <p
                    v-if="acc.last_error"
                    class="mt-1 max-w-xs break-words whitespace-normal text-xs text-danger"
                  >
                    <AlertTriangle class="mr-1 inline size-3" aria-hidden="true" />
                    {{ acc.last_error }}
                  </p>
                </td>

                <!-- Quota & Limits -->
                <td class="min-w-[220px] max-w-[280px] px-4 py-2.5 align-middle">
                  <div v-if="accountQuota(acc)" class="space-y-1.5">
                    <div class="flex items-center justify-between gap-1.5">
                      <div class="flex flex-wrap items-center gap-1">
                        <Badge
                          v-if="accountQuota(acc)?.plan"
                          variant="outline"
                          class="font-mono text-[10px] uppercase whitespace-nowrap"
                        >
                          {{ accountQuota(acc)?.plan }}
                        </Badge>
                        <span
                          v-if="accountQuota(acc)?.limit_reached"
                          class="inline-flex items-center gap-1 rounded-full border border-danger/30 bg-danger/10 px-2 py-0.5 text-[10px] font-medium text-danger whitespace-nowrap"
                        >
                          <AlertTriangle class="size-3" aria-hidden="true" />
                          {{ t('llmGateway.quota.limitReached') }}
                        </span>
                      </div>
                      <button
                        type="button"
                        class="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-50"
                        :disabled="quotaBusyKey === acc.account_key || loadingQuotas"
                        :title="t('llmGateway.quota.checkBtn')"
                        @click="refreshSingleQuota(acc)"
                      >
                        <RefreshCw
                          class="size-3"
                          :class="{ 'animate-spin': quotaBusyKey === acc.account_key }"
                          aria-hidden="true"
                        />
                      </button>
                    </div>

                    <div
                      v-if="accountQuota(acc)?.windows.length"
                      class="space-y-1.5"
                    >
                      <div
                        v-for="w in accountQuota(acc)?.windows"
                        :key="w.name"
                        class="space-y-0.5"
                      >
                        <div class="flex items-center justify-between gap-2 text-[11px]">
                          <span class="truncate font-medium text-foreground" :title="w.name">
                            {{ w.name }}
                          </span>
                          <span
                            class="shrink-0 font-mono font-semibold tabular-nums"
                            :class="quotaTextColorClass(w.remaining_percentage, w.unlimited)"
                          >
                            {{ formatWindowUsage(w) }}
                          </span>
                        </div>
                        <div class="h-1.5 w-full overflow-hidden rounded-full bg-muted">
                          <div
                            class="h-full rounded-full transition-all duration-300"
                            :class="quotaBarColorClass(w.remaining_percentage, w.unlimited)"
                            :style="{
                              width: `${w.unlimited ? 100 : Math.max(0, Math.min(100, w.remaining_percentage))}%`,
                            }"
                          />
                        </div>
                        <div
                          v-if="formatResetCountdown(w.reset_at)"
                          class="text-[10px] text-muted-foreground"
                        >
                          {{
                            t('llmGateway.quota.resetsIn', {
                              time: formatResetCountdown(w.reset_at),
                            })
                          }}
                        </div>
                      </div>
                    </div>

                    <p
                      v-else-if="accountQuota(acc)?.message"
                      class="text-xs text-muted-foreground"
                    >
                      {{ accountQuota(acc)?.message }}
                    </p>
                  </div>

                  <div v-else class="flex items-center gap-2">
                    <Button
                      variant="ghost"
                      size="sm"
                      class="h-7 px-2 text-xs text-muted-foreground whitespace-nowrap"
                      :disabled="quotaBusyKey === acc.account_key || loadingQuotas"
                      @click="refreshSingleQuota(acc)"
                    >
                      <RefreshCw
                        class="size-3.5"
                        :class="{ 'animate-spin': quotaBusyKey === acc.account_key || loadingQuotas }"
                        aria-hidden="true"
                      />
                      {{ t('llmGateway.quota.checkBtn') }}
                    </Button>
                  </div>
                </td>

                <!-- Bound Providers (single-line chips) -->
                <td class="px-4 py-2.5 align-middle">
                  <div v-if="acc.bound_providers.length" class="flex flex-wrap gap-1.5">
                    <RouterLink
                      v-for="bp in acc.bound_providers"
                      :key="`${bp.provider_key}:${bp.binding_mode}`"
                      :to="`/providers/${encodeURIComponent(bp.provider_key)}`"
                      class="inline-flex items-center gap-1 rounded-md border border-border/80 bg-surface px-2.5 py-0.5 text-xs whitespace-nowrap hover:border-studio-violet hover:text-studio-violet"
                    >
                      <span class="font-medium">{{ bp.display_name }}</span>
                      <span class="font-mono text-[10px] text-muted-foreground">
                        ({{ bp.binding_mode }})
                      </span>
                    </RouterLink>
                  </div>
                  <span v-else class="text-xs text-muted-foreground">—</span>
                </td>

                <!-- Actions (Single-line fixed layout) -->
                <td class="px-4 py-2.5 text-right align-middle whitespace-nowrap">
                  <div class="inline-flex items-center justify-end gap-1.5 whitespace-nowrap">
                    <Button
                      v-if="acc.status === 'cooling_down'"
                      variant="outline"
                      size="sm"
                      class="h-7 shrink-0 px-2.5 text-xs whitespace-nowrap"
                      :disabled="busyKey === acc.account_key"
                      @click="clearAccountCooldown(acc)"
                    >
                      <RotateCcw class="size-3.5" aria-hidden="true" />
                      {{ t('llmGateway.clearCooldown') }}
                    </Button>

                    <Button
                      variant="outline"
                      size="sm"
                      class="h-7 shrink-0 px-2.5 text-xs whitespace-nowrap"
                      :disabled="busyKey === acc.account_key"
                      @click="refreshSingleAccount(acc)"
                    >
                      <RefreshCw class="size-3.5" aria-hidden="true" />
                      {{ t('llmGateway.refreshToken') }}
                    </Button>

                    <Button
                      variant="outline"
                      size="sm"
                      class="h-7 shrink-0 px-2.5 text-xs whitespace-nowrap"
                      :disabled="busyKey === acc.account_key"
                      @click="openEditModal(acc)"
                    >
                      <Pencil class="size-3.5" aria-hidden="true" />
                      {{ t('common.edit') }}
                    </Button>

                    <Button
                      variant="outline"
                      size="sm"
                      class="h-7 shrink-0 px-2.5 text-xs whitespace-nowrap"
                      :disabled="busyKey === acc.account_key"
                      @click="toggleAccountEnabled(acc)"
                    >
                      {{ acc.enabled ? t('llmGateway.disable') : t('llmGateway.enable') }}
                    </Button>

                    <Button
                      variant="ghost"
                      size="sm"
                      class="h-7 shrink-0 px-2 text-xs text-danger"
                      :disabled="busyKey === acc.account_key || hasDirectAccountBinding(acc)"
                      :title="
                        hasDirectAccountBinding(acc)
                          ? t('llmGateway.deleteInUseHint')
                          : t('llmGateway.deleteHint')
                      "
                      @click="promptDeleteAccount(acc)"
                    >
                      <Trash2 class="size-3.5" aria-hidden="true" />
                    </Button>
                  </div>
                </td>
              </tr>
            </tbody>
          </table>
          </div>

          <!-- Models in Pool Sub-section -->
          <div
            :data-pool-models="group.vendor"
            class="border-t border-border/60 bg-surface/20 px-4 py-3"
          >
            <div class="mb-2 flex flex-wrap items-center justify-between gap-2">
              <div class="flex items-center gap-2">
                <Cpu class="size-3.5 text-studio-violet" aria-hidden="true" />
                <span class="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                  {{ t('llmGateway.pool.modelsHeading') }} ({{ group.models.length }})
                </span>
              </div>
              <Button
                variant="outline"
                size="sm"
                class="h-7 px-2.5 text-xs"
                @click="openCreatePoolModelModal(group.vendor)"
              >
                <Plus class="size-3.5" aria-hidden="true" />
                {{ t('llmGateway.pool.addModelBtn') }}
              </Button>
            </div>

            <div
              v-if="!group.models.length"
              class="rounded-lg border border-dashed border-border/70 bg-background/40 px-3 py-2 text-xs text-muted-foreground"
            >
              {{ t('llmGateway.pool.noModels') }}
            </div>

            <div v-else class="overflow-x-auto rounded-lg border border-border/70 bg-background/60">
              <table class="w-full text-left text-xs">
                <thead class="border-b border-border/60 bg-surface/40 text-[11px] uppercase text-muted-foreground">
                  <tr>
                    <th class="px-3 py-2 font-medium">{{ t('llmGateway.col.model') }}</th>
                    <th class="px-3 py-2 font-medium">{{ t('llmGateway.col.binding') }}</th>
                    <th class="px-3 py-2 font-medium">{{ t('llmGateway.col.capabilities') }}</th>
                    <th class="px-3 py-2 font-medium">{{ t('llmGateway.col.status') }}</th>
                    <th class="px-3 py-2 text-right font-medium">{{ t('llmGateway.col.actions') }}</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-border/60">
                  <tr
                    v-for="modelItem in group.models"
                    :key="modelItem.provider_key"
                    :data-pool-model-row="modelItem.provider_key"
                    class="hover:bg-accent/20"
                  >
                    <td class="px-3 py-2 align-middle">
                      <div class="flex flex-wrap items-center gap-1.5">
                        <RouterLink
                          :to="`/providers/${encodeURIComponent(modelItem.provider_key)}`"
                          class="font-semibold text-foreground hover:text-studio-violet"
                        >
                          {{ modelItem.display_name }}
                        </RouterLink>
                        <code class="rounded border border-border/70 bg-surface px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground">
                          {{ modelItem.model }}
                        </code>
                      </div>
                      <div class="mt-0.5 font-mono text-[10px] text-muted-foreground">
                        {{ modelItem.provider_key }}
                      </div>
                    </td>

                    <td class="px-3 py-2 align-middle font-mono text-[11px]">
                      <Badge variant="outline" class="font-mono text-[10px]">
                        {{ modelItem.gateway_binding }}
                      </Badge>
                    </td>

                    <td class="px-3 py-2 align-middle">
                      <div class="flex flex-wrap items-center gap-1">
                        <span
                          v-for="tag in modelItem.tags"
                          :key="tag"
                          class="rounded border border-studio-violet/30 bg-studio-violet/10 px-1.5 py-0.5 font-mono text-[10px] text-studio-violet"
                        >
                          {{ tag }}
                        </span>
                        <span
                          v-if="modelItem.reasoning_effort"
                          class="rounded border border-border/80 bg-surface px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
                        >
                          effort:{{ modelItem.reasoning_effort }}
                        </span>
                        <span
                          v-if="modelItem.max_tokens"
                          class="rounded border border-border/80 bg-surface px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
                        >
                          max:{{ modelItem.max_tokens }}
                        </span>
                      </div>
                    </td>

                    <td class="px-3 py-2 align-middle whitespace-nowrap">
                      <span
                        class="inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] font-medium"
                        :class="
                          modelItem.enabled
                            ? 'border-success/30 bg-success/10 text-success'
                            : 'border-border bg-muted/50 text-muted-foreground'
                        "
                      >
                        {{ modelItem.enabled ? t('llmGateway.status.active') : t('llmGateway.status.disabled') }}
                      </span>
                      <span
                        v-if="modelItem.template_usage_count > 0"
                        class="ml-1.5 text-[11px] text-muted-foreground"
                      >
                        · {{ modelItem.template_usage_count }} {{ t('llmGateway.col.templateUsage') }}
                      </span>
                    </td>

                    <td class="px-3 py-2 text-right align-middle whitespace-nowrap">
                      <div class="inline-flex items-center justify-end gap-1">
                        <Button
                          variant="outline"
                          size="sm"
                          class="h-7 px-2 text-xs"
                          :data-edit-pool-model="modelItem.provider_key"
                          :disabled="busyKey === modelItem.provider_key"
                          @click="openEditPoolModelModal(modelItem)"
                        >
                          <Pencil class="size-3.5" aria-hidden="true" />
                          {{ t('common.edit') }}
                        </Button>
                        <Button
                          variant="outline"
                          size="sm"
                          class="h-7 px-2 text-xs"
                          :data-toggle-pool-model="modelItem.provider_key"
                          :disabled="busyKey === modelItem.provider_key"
                          @click="togglePoolModelEnabled(modelItem)"
                        >
                          {{ modelItem.enabled ? t('llmGateway.disable') : t('llmGateway.enable') }}
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          class="h-7 px-2 text-xs text-danger"
                          :data-delete-pool-model="modelItem.provider_key"
                          :disabled="busyKey === modelItem.provider_key || modelItem.template_usage_count > 0"
                          :title="
                            modelItem.template_usage_count > 0
                              ? t('llmGateway.modelDelete.inUseHint', { count: modelItem.template_usage_count })
                              : t('llmGateway.modelDelete.hint')
                          "
                          @click="promptDeletePoolModel(modelItem)"
                        >
                          <Trash2 class="size-3.5" aria-hidden="true" />
                        </Button>
                      </div>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </div>
        </div>
      </section>
    </div>

    <!-- Compact Empty Pools Bar -->
    <div
      v-if="emptyVendorPools.length && selectedVendorFilter === 'all'"
      class="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-dashed border-border/80 bg-surface/20 px-4 py-2.5 text-xs text-muted-foreground"
    >
      <span>{{ t('llmGateway.emptyPoolsLabel') }}</span>
      <div class="flex flex-wrap items-center gap-1.5">
        <div
          v-for="emptyPool in emptyVendorPools"
          :key="emptyPool.vendor"
          class="inline-flex items-center gap-1 rounded-lg border border-border/70 bg-background/60 p-1"
        >
          <span class="px-2 font-medium text-foreground">{{ emptyPool.displayName }}</span>
          <Button
            variant="ghost"
            size="sm"
            class="h-6 px-2 text-xs whitespace-nowrap"
            :data-pool-detail="emptyPool.vendor"
            @click="openPoolDetailModal(emptyPool.vendor)"
          >
            <Boxes class="size-3" aria-hidden="true" />
            {{ t('llmGateway.pool.detailBtn') }}
          </Button>
          <Button
            variant="outline"
            size="sm"
            class="h-6 px-2 text-xs whitespace-nowrap"
            @click="openLoginModal(emptyPool.vendor)"
          >
            <Plus class="size-3" aria-hidden="true" />
            {{ t('llmGateway.addShort') }}
          </Button>
        </div>
      </div>
    </div>

    <!-- Pool Detail Modal -->
    <BaseModal
      v-model="poolDetailOpen"
      :title="
        activePoolDetail
          ? t('llmGateway.pool.detailTitle', { name: activePoolDetail.display_name })
          : t('llmGateway.pool.detailBtn')
      "
      :description="t('llmGateway.pool.detailSubtitle')"
      width-class="max-w-4xl"
    >
      <div v-if="poolDetailLoading && !activePoolDetail" class="py-8 text-center text-sm text-muted-foreground">
        {{ t('common.loading') }}
      </div>

      <div v-else-if="activePoolDetail" class="space-y-4" data-testid="pool-detail-content">
        <!-- Pool KPIs -->
        <div class="grid grid-cols-2 gap-2.5 sm:grid-cols-4">
          <div class="rounded-lg border border-border/70 bg-surface/40 p-3">
            <div class="text-[11px] uppercase text-muted-foreground">{{ t('llmGateway.col.binding') }}</div>
            <code class="mt-1 block font-mono text-xs font-semibold text-foreground">
              {{ activePoolDetail.pool_binding }}
            </code>
            <div class="mt-0.5 truncate font-mono text-[10px] text-muted-foreground" :title="activePoolDetail.default_base_url">
              {{ activePoolDetail.default_base_url }}
            </div>
          </div>

          <div class="rounded-lg border border-border/70 bg-surface/40 p-3">
            <div class="text-[11px] uppercase text-muted-foreground">{{ t('llmGateway.active') }}</div>
            <div class="mt-1 text-base font-semibold text-success">
              {{ activePoolDetail.active_accounts }} / {{ activePoolDetail.total_accounts }}
            </div>
            <div class="text-[11px] text-muted-foreground">
              {{ t('llmGateway.cooldown') }}: {{ activePoolDetail.cooling_down_accounts }}
            </div>
          </div>

          <div class="rounded-lg border border-border/70 bg-surface/40 p-3">
            <div class="text-[11px] uppercase text-muted-foreground">{{ t('llmGateway.pool.reauthAccounts') }}</div>
            <div class="mt-1 text-base font-semibold" :class="activePoolDetail.reauth_required_accounts > 0 ? 'text-danger' : 'text-foreground'">
              {{ activePoolDetail.reauth_required_accounts }}
            </div>
            <div class="text-[11px] text-muted-foreground">
              {{ t('llmGateway.pool.disabledAccounts') }}: {{ activePoolDetail.disabled_accounts }}
            </div>
          </div>

          <div class="rounded-lg border border-border/70 bg-surface/40 p-3">
            <div class="text-[11px] uppercase text-muted-foreground">{{ t('llmGateway.pool.modelsHeading') }}</div>
            <div class="mt-1 text-base font-semibold text-studio-violet">
              {{ activePoolDetail.models.length }}
            </div>
            <div class="text-[11px] text-muted-foreground">
              {{ activePoolDetail.preset_models.length }} presets
            </div>
          </div>
        </div>

        <!-- Preset Models Quick-Add -->
        <div
          v-if="activePoolDetail.preset_models.length"
          class="rounded-xl border border-border/70 bg-surface/30 p-3.5"
        >
          <div class="mb-2 flex items-center justify-between gap-2">
            <div>
              <h3 class="text-xs font-semibold uppercase tracking-wide text-foreground">
                {{ t('llmGateway.pool.presetsHeading') }}
              </h3>
              <p class="text-[11px] text-muted-foreground">
                {{ t('llmGateway.pool.presetsHint') }}
              </p>
            </div>
          </div>
          <div class="flex flex-wrap gap-2">
            <button
              v-for="preset in activePoolDetail.preset_models"
              :key="preset.model"
              type="button"
              :data-preset-model="preset.model"
              class="inline-flex items-center gap-1.5 rounded-lg border border-border/80 bg-background px-2.5 py-1.5 text-left text-xs transition-colors hover:border-studio-violet hover:bg-studio-violet/5"
              @click="openCreatePoolModelModal(activePoolDetail.vendor, preset)"
            >
              <Plus class="size-3.5 text-studio-violet" aria-hidden="true" />
              <span class="font-medium text-foreground">{{ preset.display_name }}</span>
              <code class="font-mono text-[11px] text-muted-foreground">({{ preset.model }})</code>
            </button>
          </div>
        </div>

        <!-- OAuth Accounts in Pool -->
        <div
          v-if="activePoolDetail.accounts.length"
          class="space-y-2"
          data-testid="pool-detail-accounts"
        >
          <h3 class="text-xs font-semibold uppercase tracking-wide text-foreground">
            {{ t('llmGateway.pool.accountsHeading') }} ({{ activePoolDetail.accounts.length }})
          </h3>
          <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
            <div
              v-for="acc in activePoolDetail.accounts"
              :key="acc.account_key"
              class="flex items-center justify-between gap-2 rounded-lg border border-border/70 bg-surface/30 px-3 py-2 text-xs"
            >
              <div class="min-w-0">
                <div class="truncate font-semibold text-foreground">{{ acc.label }}</div>
                <div class="truncate font-mono text-[11px] text-muted-foreground">
                  {{ acc.account_email || acc.account_key }}
                </div>
              </div>
              <span
                class="shrink-0 rounded-full border px-2 py-0.5 text-[11px] font-medium"
                :class="statusBadgeClass(acc.status)"
              >
                {{ statusLabel(acc) }}
              </span>
            </div>
          </div>
        </div>

        <!-- Configured Models in Pool -->
        <div class="space-y-2">
          <div class="flex items-center justify-between gap-2">
            <h3 class="text-xs font-semibold uppercase tracking-wide text-foreground">
              {{ t('llmGateway.pool.modelsHeading') }} ({{ activePoolDetail.models.length }})
            </h3>
            <div class="flex items-center gap-1.5">
              <Button
                variant="outline"
                size="sm"
                class="h-7 px-2.5 text-xs"
                @click="openLoginModal(activePoolDetail.vendor)"
              >
                <Plus class="size-3.5" aria-hidden="true" />
                {{ t('llmGateway.addToPool') }}
              </Button>
              <Button
                size="sm"
                class="h-7 px-2.5 text-xs"
                data-testid="pool-detail-add-model"
                @click="openCreatePoolModelModal(activePoolDetail.vendor)"
              >
                <Plus class="size-3.5" aria-hidden="true" />
                {{ t('llmGateway.pool.addModelBtn') }}
              </Button>
            </div>
          </div>

          <div
            v-if="!activePoolDetail.models.length"
            class="rounded-lg border border-dashed border-border/70 p-6 text-center text-xs text-muted-foreground"
          >
            {{ t('llmGateway.pool.noModels') }}
          </div>

          <div v-else class="overflow-x-auto rounded-lg border border-border/70">
            <table class="w-full text-left text-xs">
              <thead class="border-b border-border/60 bg-surface/40 text-[11px] uppercase text-muted-foreground">
                <tr>
                  <th class="px-3 py-2 font-medium">{{ t('llmGateway.col.model') }}</th>
                  <th class="px-3 py-2 font-medium">{{ t('llmGateway.col.binding') }}</th>
                  <th class="px-3 py-2 font-medium">{{ t('llmGateway.col.capabilities') }}</th>
                  <th class="px-3 py-2 font-medium">{{ t('llmGateway.col.status') }}</th>
                  <th class="px-3 py-2 text-right font-medium">{{ t('llmGateway.col.actions') }}</th>
                </tr>
              </thead>
              <tbody class="divide-y divide-border/60">
                <tr
                  v-for="modelItem in activePoolDetail.models"
                  :key="modelItem.provider_key"
                  class="hover:bg-accent/20"
                >
                  <td class="px-3 py-2 align-middle">
                    <div class="font-semibold text-foreground">{{ modelItem.display_name }}</div>
                    <code class="font-mono text-[11px] text-muted-foreground">{{ modelItem.model }}</code>
                  </td>
                  <td class="px-3 py-2 align-middle font-mono text-[11px]">
                    {{ modelItem.gateway_binding }}
                  </td>
                  <td class="px-3 py-2 align-middle">
                    <div class="flex flex-wrap gap-1">
                      <span
                        v-for="tag in modelItem.tags"
                        :key="tag"
                        class="rounded border border-studio-violet/30 bg-studio-violet/10 px-1.5 py-0.5 font-mono text-[10px] text-studio-violet"
                      >
                        {{ tag }}
                      </span>
                      <span
                        v-if="modelItem.reasoning_effort"
                        class="rounded border border-border/80 bg-surface px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
                      >
                        effort:{{ modelItem.reasoning_effort }}
                      </span>
                      <span
                        v-if="modelItem.max_tokens"
                        class="rounded border border-border/80 bg-surface px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
                      >
                        max:{{ modelItem.max_tokens }}
                      </span>
                    </div>
                  </td>
                  <td class="px-3 py-2 align-middle whitespace-nowrap">
                    {{ modelItem.enabled ? t('llmGateway.status.active') : t('llmGateway.status.disabled') }}
                  </td>
                  <td class="px-3 py-2 text-right align-middle whitespace-nowrap">
                    <div class="inline-flex items-center justify-end gap-1">
                      <Button
                        variant="outline"
                        size="sm"
                        class="h-7 px-2 text-xs"
                        @click="openEditPoolModelModal(modelItem)"
                      >
                        <Pencil class="size-3.5" aria-hidden="true" />
                        {{ t('common.edit') }}
                      </Button>
                      <Button
                        variant="outline"
                        size="sm"
                        class="h-7 px-2 text-xs"
                        @click="togglePoolModelEnabled(modelItem)"
                      >
                        {{ modelItem.enabled ? t('llmGateway.disable') : t('llmGateway.enable') }}
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        class="h-7 px-2 text-xs text-danger"
                        :disabled="modelItem.template_usage_count > 0"
                        @click="promptDeletePoolModel(modelItem)"
                      >
                        <Trash2 class="size-3.5" aria-hidden="true" />
                      </Button>
                    </div>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>

        <div class="flex justify-end pt-2">
          <Button type="button" variant="outline" @click="poolDetailOpen = false">
            {{ t('common.close') }}
          </Button>
        </div>
      </div>
    </BaseModal>

    <!-- Add / Edit Pool Model Modal -->
    <BaseModal
      v-model="modelModalOpen"
      :title="
        modelModalMode === 'create'
          ? t('llmGateway.modelModal.createTitle', { vendor: vendorDisplayName(modelModalVendor) })
          : t('llmGateway.modelModal.editTitle', { name: editingPoolModel?.display_name ?? '' })
      "
      :description="t('llmGateway.modelModal.description')"
      width-class="max-w-lg"
    >
      <form class="space-y-3.5" data-testid="pool-model-form" @submit.prevent="savePoolModel">
        <!-- Preset Selector -->
        <div v-if="currentModalPresets.length" class="space-y-1.5">
          <span class="block text-xs font-medium text-muted-foreground">
            {{ t('llmGateway.modelModal.presetLabel') }}
          </span>
          <div class="flex flex-wrap gap-1.5">
            <button
              v-for="preset in currentModalPresets"
              :key="preset.model"
              type="button"
              class="rounded-md border px-2 py-1 font-mono text-xs transition-colors"
              :class="
                modelForm.modelId === preset.model
                  ? 'border-studio-violet bg-studio-violet/10 text-studio-violet font-semibold'
                  : 'border-border/80 bg-surface text-muted-foreground hover:border-studio-violet/60 hover:text-foreground'
              "
              @click="applyPresetToForm(preset)"
            >
              {{ preset.model }}
            </button>
          </div>
        </div>

        <label class="block space-y-1">
          <span class="text-sm font-medium">{{ t('llmGateway.modelModal.modelId') }}</span>
          <input
            v-model="modelForm.modelId"
            data-testid="pool-model-id-input"
            class="admin-input font-mono text-sm"
            :placeholder="t('llmGateway.modelModal.modelPlaceholder')"
            maxlength="256"
            required
          />
        </label>

        <label class="block space-y-1">
          <span class="text-sm font-medium">{{ t('llmGateway.modelModal.displayName') }}</span>
          <input
            v-model="modelForm.displayName"
            data-testid="pool-model-name-input"
            class="admin-input text-sm"
            :placeholder="t('llmGateway.modelModal.displayNamePlaceholder')"
            maxlength="128"
          />
        </label>

        <label class="block space-y-1">
          <span class="text-sm font-medium">{{ t('llmGateway.modelModal.binding') }}</span>
          <select v-model="modelForm.gatewayBinding" class="admin-input text-sm">
            <option :value="`pool:${modelModalVendor}`">
              {{ t('llmGateway.modelModal.bindingPool', { binding: `pool:${modelModalVendor}` }) }}
            </option>
            <option
              v-for="acc in currentVendorAccounts"
              :key="acc.account_key"
              :value="`account:${acc.account_key}`"
            >
              {{
                t('llmGateway.modelModal.bindingAccountPrefix', {
                  label: acc.label,
                  key: acc.account_key,
                })
              }}
            </option>
          </select>
        </label>

        <label class="block space-y-1">
          <span class="text-sm font-medium">{{ t('llmGateway.modelModal.baseUrl') }}</span>
          <input
            v-model="modelForm.baseUrl"
            data-testid="pool-model-base-url-input"
            class="admin-input font-mono text-sm"
            :placeholder="t('llmGateway.modelModal.baseUrlPlaceholder')"
            maxlength="512"
          />
        </label>

        <div class="space-y-1">
          <span class="block text-sm font-medium">{{ t('llmGateway.modelModal.tags') }}</span>
          <div class="flex flex-wrap gap-1.5">
            <button
              v-for="tag in ALL_CAPABILITY_TAGS"
              :key="tag"
              type="button"
              :data-tag-option="tag"
              class="rounded-md border px-2.5 py-1 font-mono text-xs transition-colors"
              :class="
                modelForm.tags.includes(tag)
                  ? 'border-studio-violet bg-studio-violet text-white'
                  : 'border-border bg-surface text-muted-foreground hover:text-foreground'
              "
              @click="toggleFormTag(tag)"
            >
              {{ tag }}
            </button>
          </div>
        </div>

        <div class="grid grid-cols-2 gap-3">
          <label class="block space-y-1">
            <span class="text-xs font-medium">{{ t('llmGateway.modelModal.reasoningEffort') }}</span>
            <select
              v-model="modelForm.reasoningEffort"
              data-testid="pool-model-reasoning-select"
              class="admin-input text-sm"
            >
              <option value="">{{ t('llmGateway.modelModal.reasoningDefault') }}</option>
              <option value="low">low</option>
              <option value="medium">medium</option>
              <option value="high">high</option>
            </select>
          </label>

          <label class="block space-y-1">
            <span class="text-xs font-medium">{{ t('llmGateway.modelModal.timeoutMs') }}</span>
            <input
              v-model.number="modelForm.timeoutMs"
              type="number"
              min="1000"
              max="600000"
              class="admin-input text-sm"
            />
          </label>

          <label class="block space-y-1">
            <span class="text-xs font-medium">{{ t('llmGateway.modelModal.maxTokens') }}</span>
            <input
              v-model="modelForm.maxTokens"
              data-testid="pool-model-max-tokens-input"
              type="number"
              min="1"
              max="65536"
              placeholder="4096"
              class="admin-input text-sm"
            />
          </label>

          <label class="block space-y-1">
            <span class="text-xs font-medium">{{ t('llmGateway.modelModal.contextLimit') }}</span>
            <input
              v-model="modelForm.contextLimitTokens"
              type="number"
              min="4096"
              max="2000000"
              placeholder="200000"
              class="admin-input text-sm"
            />
          </label>

          <label class="block space-y-1">
            <span class="text-xs font-medium">{{ t('llmGateway.modelModal.temperature') }}</span>
            <input
              v-model="modelForm.temperaturePercent"
              data-testid="pool-model-temperature-input"
              type="number"
              min="0"
              max="200"
              placeholder="70"
              class="admin-input text-sm"
            />
          </label>
        </div>

        <label class="flex items-center gap-2 pt-1 text-sm">
          <input v-model="modelForm.enabled" type="checkbox" class="size-4 rounded border-border" />
          <span>{{ t('llmGateway.modelModal.enabled') }}</span>
        </label>

        <div class="flex justify-end gap-2 pt-2">
          <Button type="button" variant="outline" @click="modelModalOpen = false">
            {{ t('common.cancel') }}
          </Button>
          <Button type="submit" data-testid="pool-model-submit" :disabled="Boolean(busyKey)">
            {{
              modelModalMode === 'create'
                ? t('llmGateway.modelModal.createSubmit')
                : t('llmGateway.modelModal.saveSubmit')
            }}
          </Button>
        </div>
      </form>
    </BaseModal>

    <!-- Delete Pool Model Confirmation -->
    <ConfirmDialog
      v-model="deleteModelOpen"
      :title="t('llmGateway.modelDelete.title')"
      :description="
        deletingPoolModel
          ? `${deletingPoolModel.display_name} (${deletingPoolModel.model} · ${deletingPoolModel.provider_key})`
          : ''
      "
      :confirm-label="t('llmGateway.modelDelete.confirm')"
      destructive
      @confirm="confirmDeletePoolModel"
    />

    <!-- OAuth Login Modal -->
    <OAuthLoginModal
      v-model="loginModalOpen"
      :vendors="vendors"
      :initial-vendor="loginInitialVendor"
      @completed="onOAuthCompleted"
    />

    <!-- Edit Label / Priority Modal -->
    <BaseModal
      v-model="editOpen"
      :title="t('llmGateway.editModal.title')"
      :description="t('llmGateway.editModal.description')"
      width-class="max-w-md"
    >
      <form class="space-y-4" @submit.prevent="saveAccountEdit">
        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('llmGateway.editModal.label') }}</span>
          <input v-model="editLabel" class="admin-input" maxlength="128" required />
        </label>

        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('llmGateway.editModal.priority') }}</span>
          <input v-model.number="editPriority" type="number" class="admin-input" />
          <small class="block text-xs text-muted-foreground">
            {{ t('llmGateway.editModal.priorityHint') }}
          </small>
        </label>

        <div class="flex justify-end gap-2 pt-2">
          <Button type="button" variant="outline" @click="editOpen = false">{{ t('common.cancel') }}</Button>
          <Button type="submit" :disabled="Boolean(busyKey)">{{ t('common.saveChanges') }}</Button>
        </div>
      </form>
    </BaseModal>

    <!-- Delete Confirmation -->
    <ConfirmDialog
      v-model="deleteOpen"
      :title="t('llmGateway.deleteModal.title')"
      :description="
        deletingAccount
          ? `${deletingAccount.label} (${deletingAccount.account_key})`
          : ''
      "
      :confirm-label="t('llmGateway.deleteModal.confirm')"
      destructive
      @confirm="confirmDeleteAccount"
    />
  </main>
</template>

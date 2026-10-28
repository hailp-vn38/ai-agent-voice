<script setup lang="ts">
import {
  AlertTriangle,
  CheckCircle2,
  Clock,
  KeyRound,
  Pencil,
  Plus,
  RefreshCw,
  RotateCcw,
  ShieldAlert,
  Trash2,
} from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { RouterLink } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { llmGatewayApi } from '@/api/llm-gateway'
import type {
  GatewayAccountQuota,
  GatewayAccountStatus,
  GatewayAccountView,
  GatewayQuotaWindow,
  GatewayVendor,
  GatewayVendorInfo,
} from '@/api/types/llm-gateway'
import BaseModal from '@/components/admin/BaseModal.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

import OAuthLoginModal from './OAuthLoginModal.vue'

const { t } = useI18n()

const vendors = ref<GatewayVendorInfo[]>([])
const accounts = ref<GatewayAccountView[]>([])
const quotasByAccount = ref<Record<string, GatewayAccountQuota>>({})
const selectedVendorFilter = ref<GatewayVendor | 'all'>('all')
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

let loadController: AbortController | undefined
let cooldownTicker: ReturnType<typeof setInterval> | undefined

const filteredAccounts = computed(() => {
  if (selectedVendorFilter.value === 'all') return accounts.value
  return accounts.value.filter((item) => item.vendor === selectedVendorFilter.value)
})

function vendorDisplayName(vendor: GatewayVendor): string {
  return (
    vendors.value.find((item) => item.vendor === vendor)?.display_name ??
    vendor.replace(/_/g, ' ')
  )
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
  <main class="mx-auto max-w-7xl space-y-5">
    <!-- Header -->
    <header class="flex flex-wrap items-start justify-between gap-4">
      <div class="flex items-start gap-3">
        <span
          class="flex size-10 shrink-0 items-center justify-center rounded-xl border border-studio-violet/20 bg-studio-violet/10 text-studio-violet"
        >
          <KeyRound class="size-5" aria-hidden="true" />
        </span>
        <div>
          <h1 class="text-xl font-semibold tracking-tight">
            {{ t('llmGateway.title') }}
          </h1>
          <p class="mt-1 text-sm text-muted-foreground">
            {{ t('llmGateway.subtitle') }}
          </p>
        </div>
      </div>

      <div class="flex flex-wrap items-center gap-2">
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

    <!-- Vendor Pool Summary Cards -->
    <section aria-label="Vendor Pools" class="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
      <div
        v-for="v in vendors"
        :key="v.vendor"
        class="studio-panel flex flex-col justify-between p-4 transition-colors"
        :class="
          selectedVendorFilter === v.vendor
            ? 'ring-2 ring-studio-violet'
            : 'hover:border-studio-violet/40'
        "
      >
        <div>
          <div class="flex items-start justify-between gap-2">
            <div>
              <h2 class="font-semibold text-sm">{{ v.display_name }}</h2>
              <code class="mt-0.5 block font-mono text-xs text-muted-foreground">
                {{ v.pool_binding }}
              </code>
            </div>
            <Badge variant="outline" class="font-mono text-[10px]">
              {{ v.login_mode === 'device_code' ? 'Device Code' : 'PKCE' }}
            </Badge>
          </div>

          <div class="mt-4 grid grid-cols-3 gap-2 text-center">
            <div class="rounded-lg border border-border/60 bg-surface/50 px-2 py-1.5">
              <p class="text-[11px] text-muted-foreground">{{ t('llmGateway.total') }}</p>
              <p class="font-mono text-sm font-semibold tabular-nums">{{ v.total_accounts }}</p>
            </div>
            <div class="rounded-lg border border-border/60 bg-surface/50 px-2 py-1.5">
              <p class="text-[11px] text-muted-foreground">{{ t('llmGateway.active') }}</p>
              <p class="font-mono text-sm font-semibold tabular-nums text-success">
                {{ v.active_accounts }}
              </p>
            </div>
            <div class="rounded-lg border border-border/60 bg-surface/50 px-2 py-1.5">
              <p class="text-[11px] text-muted-foreground">{{ t('llmGateway.cooldown') }}</p>
              <p class="font-mono text-sm font-semibold tabular-nums text-warning">
                {{ v.cooling_down_accounts }}
              </p>
            </div>
          </div>
        </div>

        <div class="mt-4 flex items-center justify-between gap-2 border-t border-border/60 pt-3">
          <button
            type="button"
            class="text-xs font-medium text-studio-violet hover:underline"
            @click="
              selectedVendorFilter = selectedVendorFilter === v.vendor ? 'all' : v.vendor
            "
          >
            {{
              selectedVendorFilter === v.vendor
                ? t('llmGateway.showAll')
                : t('llmGateway.filterByVendor')
            }}
          </button>
          <Button variant="ghost" size="sm" class="h-7 px-2 text-xs" @click="openLoginModal(v.vendor)">
            <Plus class="size-3.5" aria-hidden="true" />
            {{ t('llmGateway.addShort') }}
          </Button>
        </div>
      </div>
    </section>

    <!-- Filter Bar & Account Table -->
    <section class="studio-panel overflow-hidden">
      <div class="flex flex-wrap items-center justify-between gap-3 border-b px-4 py-3 sm:px-5">
        <div class="flex flex-wrap items-center gap-1.5">
          <button
            type="button"
            class="rounded-lg px-2.5 py-1 text-xs font-medium transition-colors"
            :class="
              selectedVendorFilter === 'all'
                ? 'bg-studio-violet text-white'
                : 'text-muted-foreground hover:bg-accent'
            "
            @click="selectedVendorFilter = 'all'"
          >
            {{ t('llmGateway.allAccounts') }} ({{ accounts.length }})
          </button>
          <button
            v-for="v in vendors"
            :key="v.vendor"
            type="button"
            class="rounded-lg px-2.5 py-1 text-xs font-medium transition-colors"
            :class="
              selectedVendorFilter === v.vendor
                ? 'bg-studio-violet text-white'
                : 'text-muted-foreground hover:bg-accent'
            "
            @click="selectedVendorFilter = v.vendor"
          >
            {{ v.display_name }} ({{ v.total_accounts }})
          </button>
        </div>
      </div>

      <div v-if="loading && !accounts.length" class="p-10 text-center text-sm text-muted-foreground">
        {{ t('llmGateway.loading') }}
      </div>

      <div
        v-else-if="!filteredAccounts.length"
        class="space-y-3 p-10 text-center text-sm text-muted-foreground"
      >
        <p>{{ t('llmGateway.empty') }}</p>
        <Button size="sm" variant="outline" @click="openLoginModal(selectedVendorFilter === 'all' ? undefined : selectedVendorFilter)">
          <Plus class="size-4" aria-hidden="true" />
          {{ t('llmGateway.emptyCta') }}
        </Button>
      </div>

      <div v-else class="overflow-x-auto">
        <table class="w-full text-left text-sm">
          <thead class="border-b bg-surface/60 text-xs uppercase text-muted-foreground">
            <tr>
              <th class="px-4 py-3 font-medium">{{ t('llmGateway.col.account') }}</th>
              <th class="px-4 py-3 font-medium">{{ t('llmGateway.col.vendor') }}</th>
              <th class="px-4 py-3 font-medium">{{ t('llmGateway.col.status') }}</th>
              <th class="px-4 py-3 font-medium">{{ t('llmGateway.col.quota') }}</th>
              <th class="px-4 py-3 font-medium">{{ t('llmGateway.col.priority') }}</th>
              <th class="px-4 py-3 font-medium">{{ t('llmGateway.col.boundProviders') }}</th>
              <th class="px-4 py-3 text-right font-medium">{{ t('llmGateway.col.actions') }}</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-border/70">
            <tr
              v-for="acc in filteredAccounts"
              :key="acc.account_key"
              class="hover:bg-accent/20"
            >
              <!-- Label, Email & Credential Metadata -->
              <td class="px-4 py-3 align-top">
                <div class="font-medium text-foreground">{{ acc.label }}</div>
                <div v-if="acc.account_email" class="text-xs text-muted-foreground">
                  {{ acc.account_email }}
                </div>
                <code class="mt-0.5 block font-mono text-[11px] text-muted-foreground">
                  account:{{ acc.account_key }}
                </code>
                <div
                  v-if="maskedCredentialSummary(acc)"
                  class="mt-0.5 font-mono text-[11px] text-muted-foreground"
                >
                  {{ maskedCredentialSummary(acc) }}
                </div>
              </td>

              <!-- Vendor -->
              <td class="px-4 py-3 align-top">
                <Badge variant="outline">{{ vendorDisplayName(acc.vendor) }}</Badge>
                <div class="mt-1 font-mono text-[11px] text-muted-foreground">
                  pool:{{ acc.vendor }}
                </div>
              </td>

              <!-- Status -->
              <td class="px-4 py-3 align-top">
                <span
                  class="inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium"
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
                  class="mt-1 max-w-xs break-words text-xs text-danger"
                >
                  <AlertTriangle class="mr-1 inline size-3" aria-hidden="true" />
                  {{ acc.last_error }}
                </p>
              </td>

              <!-- Quota & Limits -->
              <td class="min-w-[230px] max-w-[300px] px-4 py-3 align-top">
                <div v-if="accountQuota(acc)" class="space-y-2">
                  <div class="flex flex-wrap items-center justify-between gap-1.5">
                    <div class="flex flex-wrap items-center gap-1">
                      <Badge
                        v-if="accountQuota(acc)?.plan"
                        variant="outline"
                        class="font-mono text-[10px] uppercase"
                      >
                        {{ accountQuota(acc)?.plan }}
                      </Badge>
                      <span
                        v-if="accountQuota(acc)?.limit_reached"
                        class="inline-flex items-center gap-1 rounded-full border border-danger/30 bg-danger/10 px-2 py-0.5 text-[10px] font-medium text-danger"
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
                    class="h-7 px-2 text-xs text-muted-foreground"
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

              <!-- Priority -->
              <td class="px-4 py-3 align-top font-mono text-xs tabular-nums">
                {{ acc.priority }}
              </td>

              <!-- Bound Providers -->
              <td class="px-4 py-3 align-top">
                <div v-if="acc.bound_providers.length" class="flex flex-wrap gap-1.5">
                  <RouterLink
                    v-for="bp in acc.bound_providers"
                    :key="`${bp.provider_key}:${bp.binding_mode}`"
                    :to="`/providers/${encodeURIComponent(bp.provider_key)}`"
                    class="inline-flex items-center gap-1 rounded-md border border-border/80 bg-surface px-2 py-0.5 text-xs hover:border-studio-violet hover:text-studio-violet"
                  >
                    <span class="font-medium">{{ bp.display_name }}</span>
                    <span class="font-mono text-[10px] text-muted-foreground">
                      ({{ bp.binding_mode }})
                    </span>
                  </RouterLink>
                </div>
                <span v-else class="text-xs text-muted-foreground">—</span>
              </td>

              <!-- Actions -->
              <td class="px-4 py-3 align-top text-right">
                <div class="flex flex-wrap items-center justify-end gap-1.5">
                  <Button
                    v-if="acc.status === 'cooling_down'"
                    variant="outline"
                    size="sm"
                    class="h-7 px-2 text-xs"
                    :disabled="busyKey === acc.account_key"
                    @click="clearAccountCooldown(acc)"
                  >
                    <RotateCcw class="size-3.5" aria-hidden="true" />
                    {{ t('llmGateway.clearCooldown') }}
                  </Button>

                  <Button
                    variant="outline"
                    size="sm"
                    class="h-7 px-2 text-xs"
                    :disabled="busyKey === acc.account_key"
                    @click="refreshSingleAccount(acc)"
                  >
                    <RefreshCw class="size-3.5" aria-hidden="true" />
                    {{ t('llmGateway.refreshToken') }}
                  </Button>

                  <Button
                    variant="outline"
                    size="sm"
                    class="h-7 px-2 text-xs"
                    :disabled="busyKey === acc.account_key"
                    @click="openEditModal(acc)"
                  >
                    <Pencil class="size-3.5" aria-hidden="true" />
                    {{ t('common.edit') }}
                  </Button>

                  <Button
                    variant="outline"
                    size="sm"
                    class="h-7 px-2 text-xs"
                    :disabled="busyKey === acc.account_key"
                    @click="toggleAccountEnabled(acc)"
                  >
                    {{ acc.enabled ? t('llmGateway.disable') : t('llmGateway.enable') }}
                  </Button>

                  <Button
                    variant="ghost"
                    size="sm"
                    class="h-7 px-2 text-xs text-danger"
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
    </section>

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

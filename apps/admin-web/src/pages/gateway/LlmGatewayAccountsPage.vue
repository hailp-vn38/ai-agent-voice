<script setup lang="ts">
import { ArrowLeft, ChevronRight, Plus, RefreshCw, Search } from '@lucide/vue'
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { llmGatewayApi } from '@/api/llm-gateway'
import type {
  GatewayAccountQuota,
  GatewayAccountStatus,
  GatewayAccountView,
  GatewayVendor,
  GatewayVendorInfo,
} from '@/api/types/llm-gateway'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useErrorToast } from '@/composables/useNotify'
import { primaryQuotaPercent, quotaBarColorClass, statusBadgeClass } from './llmGatewayUi'

const router = useRouter()
const { t } = useI18n()

const vendors = ref<GatewayVendorInfo[]>([])
const accounts = ref<GatewayAccountView[]>([])
const quotasByAccount = ref<Record<string, GatewayAccountQuota>>({})
const searchQuery = ref('')
const statusFilter = ref<'all' | GatewayAccountStatus>('all')
const loading = ref(true)
const loadingQuotas = ref(false)
const errorMessage = ref('')
useErrorToast(() => errorMessage.value, { retry: () => load() })

function vendorName(vendor: GatewayVendor): string {
  return vendors.value.find((v) => v.vendor === vendor)?.display_name ?? vendor
}

function statusLabel(acc: GatewayAccountView): string {
  switch (acc.status) {
    case 'active':
      return t('llmGateway.status.active')
    case 'cooling_down':
      return t('llmGateway.status.coolingDown')
    case 'reauth_required':
      return t('llmGateway.status.reauthRequired')
    case 'disabled':
    default:
      return t('llmGateway.status.disabled')
  }
}

function accountQuotaPct(acc: GatewayAccountView): number | null {
  const q = quotasByAccount.value[acc.account_key] ?? acc.quota
  return primaryQuotaPercent(q)
}

const filteredAccounts = computed(() => {
  const q = searchQuery.value.trim().toLowerCase()
  return accounts.value.filter((acc) => {
    if (statusFilter.value !== 'all' && acc.status !== statusFilter.value) {
      return false
    }
    if (!q) return true
    const haystack = [
      acc.label,
      acc.account_email ?? '',
      acc.account_key,
      acc.vendor,
      vendorName(acc.vendor),
    ]
      .join(' ')
      .toLowerCase()
    return haystack.includes(q)
  })
})

async function fetchAllQuotas(force = false) {
  if (!accounts.value.length) return
  loadingQuotas.value = true
  try {
    const res = await llmGatewayApi.listQuotas({ force })
    const map: Record<string, GatewayAccountQuota> = { ...quotasByAccount.value }
    for (const q of res.quotas) {
      map[q.account_key] = q
    }
    quotasByAccount.value = map
  } catch {
    /* Non-blocking quota refresh */
  } finally {
    loadingQuotas.value = false
  }
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
    if (accountRes.accounts.length) {
      void fetchAllQuotas(false)
    }
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    loading.value = false
  }
}

function goAccount(accountKey: string) {
  void router.push({ name: 'llm-gateway-account', params: { accountKey } })
}

onMounted(() => {
  void load()
})
</script>

<template>
  <section class="space-y-4" data-testid="llm-gateway-accounts-page">
    <div class="flex flex-wrap items-center justify-between gap-2">
      <button
        type="button"
        data-detail-back
        class="inline-flex items-center gap-1.5 rounded-lg border border-border/80 bg-surface/60 px-2.5 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:border-studio-violet/40 hover:text-foreground"
        @click="router.push({ name: 'llm-gateway' })"
      >
        <ArrowLeft class="size-3.5" aria-hidden="true" />
        {{ t('llmGateway.backToGateway') }}
      </button>
    </div>

    <header class="flex flex-wrap items-start justify-between gap-3">
      <div class="space-y-1">
        <p class="text-[11px] font-bold tracking-[0.14em] text-studio-violet uppercase">
          {{ t('llmGateway.accountsPage.eyebrow') }}
        </p>
        <h1 class="text-xl font-bold tracking-tight sm:text-2xl">
          {{ t('llmGateway.accountsPage.title') }}
        </h1>
        <p class="max-w-2xl text-xs text-muted-foreground sm:text-sm">
          {{ t('llmGateway.accountsPage.subtitle') }}
        </p>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <Button
          variant="outline"
          size="sm"
          :disabled="loadingQuotas || loading"
          @click="fetchAllQuotas(true)"
        >
          <RefreshCw class="size-4" :class="{ 'animate-spin': loadingQuotas }" aria-hidden="true" />
          {{ t('llmGateway.refreshQuotas') }}
        </Button>
        <Button size="sm" @click="router.push({ name: 'llm-gateway-connect' })">
          <Plus class="size-4" aria-hidden="true" />
          {{ t('llmGateway.hub.connectCta') }}
        </Button>
      </div>
    </header>

    <div class="studio-panel overflow-hidden">
      <div class="space-y-3 border-b border-border/70 p-4">
        <div class="flex flex-wrap items-center gap-2.5">
          <div class="relative min-w-[220px] flex-1">
            <Search
              class="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
              aria-hidden="true"
            />
            <input
              v-model="searchQuery"
              data-input="account-search"
              class="admin-input pl-9 text-xs"
              :placeholder="t('llmGateway.accountsPage.searchPlaceholder')"
            />
          </div>

          <select
            v-model="statusFilter"
            data-change="account-filter"
            class="admin-input w-auto min-w-[140px] text-xs"
          >
            <option value="all">{{ t('llmGateway.accountsPage.allStatuses') }}</option>
            <option value="active">active</option>
            <option value="cooling_down">Cooldown</option>
            <option value="reauth_required">Re-auth</option>
            <option value="disabled">disabled</option>
          </select>
        </div>

        <div class="flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
          <span>
            {{
              t('llmGateway.accountsPage.countSummary', {
                shown: filteredAccounts.length,
                total: accounts.length,
              })
            }}
          </span>
        </div>
      </div>

      <div class="overflow-x-auto">
        <table class="w-full text-left text-xs">
          <thead class="border-b border-border/60 bg-surface/30 text-[11px] uppercase text-muted-foreground">
            <tr>
              <th class="px-4 py-2.5 font-semibold">ACCOUNT</th>
              <th class="px-4 py-2.5 font-semibold">STATE</th>
              <th class="px-4 py-2.5 font-semibold">PRIORITY</th>
              <th class="px-4 py-2.5 font-semibold">QUOTA</th>
              <th class="px-4 py-2.5 font-semibold">MODEL BINDINGS</th>
              <th class="px-4 py-2.5 text-right font-semibold" />
            </tr>
          </thead>
          <tbody class="divide-y divide-border/60">
            <tr
              v-for="acc in filteredAccounts"
              :key="acc.account_key"
              :data-account="acc.account_key"
              :data-select-account="acc.account_key"
              role="button"
              tabindex="0"
              class="cursor-pointer transition-colors hover:bg-accent/25"
              @click="goAccount(acc.account_key)"
              @keydown.enter.prevent="goAccount(acc.account_key)"
              @keydown.space.prevent="goAccount(acc.account_key)"
            >
              <td class="px-4 py-2.5 align-middle">
                <div class="font-bold text-foreground">{{ acc.label }}</div>
                <div class="text-[11px] text-muted-foreground">
                  {{ vendorName(acc.vendor) }} · {{ acc.account_email || acc.account_key }}
                </div>
              </td>
              <td class="px-4 py-2.5 align-middle">
                <span
                  class="inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] font-medium"
                  :class="statusBadgeClass(acc.status)"
                >
                  {{ statusLabel(acc) }}
                </span>
              </td>
              <td class="px-4 py-2.5 align-middle font-mono">P{{ acc.priority }}</td>
              <td class="min-w-[140px] px-4 py-2.5 align-middle">
                <template v-if="accountQuotaPct(acc) !== null">
                  <div class="mb-1 flex items-center justify-between text-[11px]">
                    <span class="text-muted-foreground">Remaining</span>
                    <span class="font-mono font-semibold">{{ accountQuotaPct(acc) }}%</span>
                  </div>
                  <div class="h-1.5 w-full overflow-hidden rounded-full bg-muted">
                    <div
                      class="h-full rounded-full"
                      :class="quotaBarColorClass(accountQuotaPct(acc) ?? 100, false)"
                      :style="{ width: `${accountQuotaPct(acc)}%` }"
                    />
                  </div>
                </template>
                <span v-else class="text-[11px] text-muted-foreground">—</span>
              </td>
              <td class="px-4 py-2.5 align-middle">
                <Badge variant="outline" class="font-mono text-[10px]">
                  {{ acc.bound_providers.length }} bindings
                </Badge>
              </td>
              <td class="px-4 py-2.5 text-right align-middle">
                <Button
                  variant="outline"
                  size="sm"
                  class="h-7 px-2.5 text-xs"
                  @click.stop="goAccount(acc.account_key)"
                >
                  {{ t('llmGateway.hub.detailLink') }}
                  <ChevronRight class="size-3.5" aria-hidden="true" />
                </Button>
              </td>
            </tr>
            <tr v-if="!loading && !filteredAccounts.length">
              <td colspan="6" class="px-4 py-8 text-center text-xs text-muted-foreground">
                {{ t('llmGateway.empty') }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </section>
</template>

<script setup lang="ts">
import {
  AlertTriangle,
  ArrowRight,
  ChevronRight,
  Clock,
  Cpu,
  Gauge,
  KeyRound,
  Layers,
  Network,
  Plus,
  RefreshCw,
  User,
} from '@lucide/vue'
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { llmGatewayApi } from '@/api/llm-gateway'
import type {
  GatewayAccountQuota,
  GatewayAccountView,
  GatewayPoolModelView,
  GatewayVendor,
  GatewayVendorInfo,
} from '@/api/types/llm-gateway'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useErrorToast } from '@/composables/useNotify'
import {
  primaryQuotaPercent,
  quotaBarColorClass,
  statusBadgeClass,
  vendorMeta,
} from './llmGatewayUi'

const router = useRouter()
const { t } = useI18n()

const vendors = ref<GatewayVendorInfo[]>([])
const accounts = ref<GatewayAccountView[]>([])
const quotasByAccount = ref<Record<string, GatewayAccountQuota>>({})
const loading = ref(true)
const loadingQuotas = ref(false)
const errorMessage = ref('')
useErrorToast(() => errorMessage.value, { retry: () => load() })

interface PoolSummaryCard {
  vendor: GatewayVendor
  displayName: string
  poolBinding: string
  activeAccounts: number
  totalAccounts: number
  issueCount: number
  models: GatewayPoolModelView[]
  tags: string[]
}

const poolCards = computed<PoolSummaryCard[]>(() =>
  vendors.value.map((v) => {
    const vendorAccounts = accounts.value.filter((a) => a.vendor === v.vendor)
    const activeAccounts = vendorAccounts.length
      ? vendorAccounts.filter((a) => a.status === 'active').length
      : v.active_accounts
    const totalAccounts = vendorAccounts.length || v.total_accounts
    const issueCount = vendorAccounts.length
      ? vendorAccounts.filter(
          (a) => a.status === 'cooling_down' || a.status === 'reauth_required',
        ).length
      : v.cooling_down_accounts
    const models = v.models ?? []
    const tagSet = new Set<string>()
    for (const m of models) {
      for (const tag of m.tags) tagSet.add(tag)
    }
    return {
      vendor: v.vendor,
      displayName: v.display_name,
      poolBinding: v.pool_binding,
      activeAccounts,
      totalAccounts,
      issueCount,
      models,
      tags: Array.from(tagSet),
    }
  }),
)

const activeAccountsCount = computed(
  () => accounts.value.filter((a) => a.status === 'active').length,
)

const enabledModelsCount = computed(() => {
  let count = 0
  for (const v of vendors.value) {
    for (const m of v.models ?? []) {
      if (m.enabled) count += 1
    }
  }
  return count
})

const issueAccounts = computed(() =>
  accounts.value.filter(
    (a) => a.status === 'cooling_down' || a.status === 'reauth_required',
  ),
)

const recentAccounts = computed(() => accounts.value.slice(0, 5))

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
    /* Non-blocking */
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

function goPool(vendor: GatewayVendor) {
  void router.push({ name: 'llm-gateway-pool', params: { vendor } })
}

function goAccount(accountKey: string) {
  void router.push({ name: 'llm-gateway-account', params: { accountKey } })
}

onMounted(() => {
  void load()
})
</script>

<template>
  <section class="space-y-4" data-testid="llm-gateway-hub-page">
    <!-- Header -->
    <header class="flex flex-wrap items-start justify-between gap-3">
      <div class="space-y-1">
        <p class="text-[11px] font-bold tracking-[0.14em] text-studio-violet uppercase">
          {{ t('llmGateway.hub.eyebrow') }}
        </p>
        <h1 class="text-xl font-bold tracking-tight sm:text-2xl">
          {{ t('llmGateway.hub.title') }}
        </h1>
        <p class="max-w-2xl text-xs text-muted-foreground sm:text-sm">
          {{ t('llmGateway.hub.subtitle') }}
        </p>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <Button
          variant="outline"
          size="sm"
          data-action="refresh"
          :disabled="loading"
          @click="load"
        >
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" aria-hidden="true" />
          {{ t('common.refresh') }}
        </Button>
        <Button
          size="sm"
          data-action="go-connect"
          @click="router.push({ name: 'llm-gateway-connect' })"
        >
          <Plus class="size-4" aria-hidden="true" />
          {{ t('llmGateway.hub.connectCta') }}
        </Button>
      </div>
    </header>

    <!-- Hero Control Center Banner -->
    <section
      data-testid="llm-gateway-hero"
      class="studio-panel flex flex-wrap items-center justify-between gap-3 border-studio-violet/25 bg-gradient-to-br from-studio-violet/10 via-surface/60 to-teal-500/5 p-4"
    >
      <div class="space-y-1">
        <p class="text-[10px] font-bold tracking-[0.14em] text-studio-violet uppercase">
          {{ t('llmGateway.hub.heroEyebrow') }}
        </p>
        <strong class="block text-sm font-bold text-foreground sm:text-base">
          {{ t('llmGateway.hub.heroTitle') }}
        </strong>
        <p class="max-w-2xl text-xs text-muted-foreground">
          {{ t('llmGateway.hub.heroDesc') }}
        </p>
      </div>
      <div class="flex size-12 shrink-0 items-center justify-center rounded-2xl border border-studio-violet/30 bg-studio-violet/12 text-studio-violet">
        <Network class="size-6" aria-hidden="true" />
      </div>
    </section>

    <!-- 4 Clickable KPI Cards -->
    <div class="grid grid-cols-2 gap-3 lg:grid-cols-4">
      <article
        data-kpi-card="pools"
        role="button"
        tabindex="0"
        class="studio-panel flex cursor-pointer items-center gap-3 p-3.5 transition-colors hover:border-studio-violet/50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        @click="router.push({ name: 'llm-gateway-pools' })"
        @keydown.enter.prevent="router.push({ name: 'llm-gateway-pools' })"
        @keydown.space.prevent="router.push({ name: 'llm-gateway-pools' })"
      >
        <span class="inline-flex size-10 shrink-0 items-center justify-center rounded-xl border border-studio-violet/25 bg-studio-violet/10 text-studio-violet">
          <Layers class="size-4" aria-hidden="true" />
        </span>
        <div class="min-w-0">
          <div class="text-xl font-extrabold tracking-tight text-foreground">
            {{ vendors.length }}
          </div>
          <div class="truncate text-xs font-bold text-foreground">
            {{ t('llmGateway.hub.kpiPools') }}
          </div>
          <div class="truncate text-[11px] text-muted-foreground">
            {{ t('llmGateway.hub.kpiPoolsDesc') }}
          </div>
        </div>
      </article>

      <article
        data-kpi-card="accounts"
        role="button"
        tabindex="0"
        class="studio-panel flex cursor-pointer items-center gap-3 p-3.5 transition-colors hover:border-studio-violet/50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        @click="router.push({ name: 'llm-gateway-accounts' })"
        @keydown.enter.prevent="router.push({ name: 'llm-gateway-accounts' })"
        @keydown.space.prevent="router.push({ name: 'llm-gateway-accounts' })"
      >
        <span class="inline-flex size-10 shrink-0 items-center justify-center rounded-xl border border-studio-violet/25 bg-studio-violet/10 text-studio-violet">
          <User class="size-4" aria-hidden="true" />
        </span>
        <div class="min-w-0">
          <div class="text-xl font-extrabold tracking-tight text-foreground">
            {{ accounts.length }}
          </div>
          <div class="truncate text-xs font-bold text-foreground">
            {{ t('llmGateway.hub.kpiAccounts') }}
          </div>
          <div class="truncate text-[11px] text-muted-foreground">
            {{ t('llmGateway.hub.kpiAccountsDesc', { count: activeAccountsCount }) }}
          </div>
        </div>
      </article>

      <article
        data-kpi-card="models"
        role="button"
        tabindex="0"
        class="studio-panel flex cursor-pointer items-center gap-3 p-3.5 transition-colors hover:border-studio-violet/50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        @click="router.push({ name: 'llm-gateway-models' })"
        @keydown.enter.prevent="router.push({ name: 'llm-gateway-models' })"
        @keydown.space.prevent="router.push({ name: 'llm-gateway-models' })"
      >
        <span class="inline-flex size-10 shrink-0 items-center justify-center rounded-xl border border-studio-violet/25 bg-studio-violet/10 text-studio-violet">
          <Cpu class="size-4" aria-hidden="true" />
        </span>
        <div class="min-w-0">
          <div class="text-xl font-extrabold tracking-tight text-foreground">
            {{ enabledModelsCount }}
          </div>
          <div class="truncate text-xs font-bold text-foreground">
            {{ t('llmGateway.hub.kpiModels') }}
          </div>
          <div class="truncate text-[11px] text-muted-foreground">
            {{ t('llmGateway.hub.kpiModelsDesc') }}
          </div>
        </div>
      </article>

      <article
        data-kpi-card="quotas"
        role="button"
        tabindex="0"
        class="studio-panel flex cursor-pointer items-center gap-3 p-3.5 transition-colors hover:border-studio-violet/50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        @click="router.push({ name: 'llm-gateway-quotas' })"
        @keydown.enter.prevent="router.push({ name: 'llm-gateway-quotas' })"
        @keydown.space.prevent="router.push({ name: 'llm-gateway-quotas' })"
      >
        <span class="inline-flex size-10 shrink-0 items-center justify-center rounded-xl border border-studio-violet/25 bg-studio-violet/10 text-studio-violet">
          <AlertTriangle class="size-4" aria-hidden="true" />
        </span>
        <div class="min-w-0">
          <div class="text-xl font-extrabold tracking-tight text-foreground">
            {{ issueAccounts.length }}
          </div>
          <div class="truncate text-xs font-bold text-foreground">
            {{ t('llmGateway.hub.kpiIssues') }}
          </div>
          <div class="truncate text-[11px] text-muted-foreground">
            {{ t('llmGateway.hub.kpiIssuesDesc') }}
          </div>
        </div>
      </article>
    </div>

    <!-- Loading Skeleton -->
    <div
      v-if="loading"
      data-llm-gateway-skeleton
      role="status"
      aria-busy="true"
      :aria-label="t('common.loading')"
      class="grid gap-4 lg:grid-cols-[minmax(0,1fr)_315px]"
    >
      <div class="space-y-3.5">
        <div class="grid gap-3.5 sm:grid-cols-2">
          <div
            v-for="slot in 4"
            :key="slot"
            class="studio-panel card-shimmer h-40 p-4"
            aria-hidden="true"
          />
        </div>
        <div class="studio-panel card-shimmer h-48 p-4" aria-hidden="true" />
      </div>
      <div class="space-y-3.5">
        <div class="studio-panel card-shimmer h-44 p-4" aria-hidden="true" />
        <div class="studio-panel card-shimmer h-44 p-4" aria-hidden="true" />
      </div>
    </div>

    <!-- Main 2-Column Hub Layout -->
    <div v-else class="grid gap-4 lg:grid-cols-[minmax(0,1fr)_315px]">
      <!-- Left Column -->
      <div class="space-y-4">
        <!-- Vendor Pools Section -->
        <div class="space-y-2.5">
          <div class="flex flex-wrap items-end justify-between gap-2">
            <div>
              <h2 class="text-sm font-bold text-foreground sm:text-base">
                {{ t('llmGateway.hub.poolsTitle') }}
              </h2>
              <p class="text-xs text-muted-foreground">
                {{ t('llmGateway.hub.poolsSubtitle') }}
              </p>
            </div>
            <Button
              variant="outline"
              size="sm"
              class="h-7 px-2.5 text-xs"
              data-nav="pools"
              @click="router.push({ name: 'llm-gateway-pools' })"
            >
              {{ t('llmGateway.hub.allPoolsBtn') }}
              <ArrowRight class="size-3.5" aria-hidden="true" />
            </Button>
          </div>

          <div class="grid gap-3.5 sm:grid-cols-2">
            <article
              v-for="pool in poolCards"
              :key="pool.vendor"
              :data-pool="pool.vendor"
              :data-pool-group="pool.vendor"
              role="button"
              tabindex="0"
              class="studio-panel group flex cursor-pointer flex-col justify-between p-4 transition-colors hover:border-studio-violet/50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
              @click="goPool(pool.vendor)"
              @keydown.enter.prevent="goPool(pool.vendor)"
              @keydown.space.prevent="goPool(pool.vendor)"
            >
              <div>
                <div class="flex items-start justify-between gap-2.5">
                  <div class="flex items-start gap-2.5">
                    <span
                      class="inline-flex size-9 shrink-0 items-center justify-center rounded-xl border font-mono text-xs font-extrabold"
                      :class="vendorMeta(pool.vendor).logoClass"
                    >
                      {{ vendorMeta(pool.vendor).short }}
                    </span>
                    <div class="min-w-0">
                      <h3 class="truncate text-sm font-bold text-foreground">
                        {{ pool.displayName }}
                      </h3>
                      <p class="font-mono text-xs text-muted-foreground">
                        {{ pool.poolBinding }}
                      </p>
                    </div>
                  </div>

                  <span
                    class="inline-flex shrink-0 items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] font-semibold"
                    :class="
                      pool.issueCount > 0
                        ? 'border-warning/30 bg-warning/10 text-warning'
                        : 'border-success/30 bg-success/10 text-success'
                    "
                  >
                    <span
                      class="size-1.5 rounded-full"
                      :class="pool.issueCount > 0 ? 'bg-warning' : 'bg-success'"
                    />
                    {{
                      pool.issueCount > 0
                        ? t('llmGateway.hub.issuesCount', { count: pool.issueCount })
                        : t('llmGateway.hub.healthy')
                    }}
                  </span>
                </div>

                <div class="my-3 h-px bg-border/60" />

                <div class="grid grid-cols-2 gap-2.5">
                  <div class="rounded-lg border border-border/60 bg-surface/40 p-2.5">
                    <div class="text-[10px] font-semibold tracking-wider text-muted-foreground uppercase">
                      {{ t('llmGateway.hub.accountsLabel') }}
                    </div>
                    <div class="mt-1 text-base font-bold text-foreground">
                      {{ pool.activeAccounts }}
                      <span class="text-xs font-normal text-muted-foreground">
                        / {{ pool.totalAccounts }}
                      </span>
                    </div>
                  </div>

                  <div class="rounded-lg border border-border/60 bg-surface/40 p-2.5">
                    <div class="text-[10px] font-semibold tracking-wider text-muted-foreground uppercase">
                      {{ t('llmGateway.hub.poolModelsLabel') }}
                    </div>
                    <div
                      class="mt-1 text-base font-bold text-foreground"
                      :data-pool-model-count="pool.vendor"
                    >
                      {{ pool.models.length }}
                    </div>
                  </div>
                </div>
              </div>

              <div class="mt-3 flex items-center justify-between gap-2 pt-1">
                <div class="flex flex-wrap items-center gap-1">
                  <template v-if="pool.tags.length">
                    <span
                      v-for="tag in pool.tags.slice(0, 2)"
                      :key="tag"
                      class="rounded border border-border/70 bg-surface px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
                    >
                      {{ tag }}
                    </span>
                    <span
                      v-if="pool.tags.length > 2"
                      class="rounded border border-border/70 bg-surface px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
                    >
                      +{{ pool.tags.length - 2 }}
                    </span>
                  </template>
                  <span v-else class="text-xs text-muted-foreground">
                    {{ t('llmGateway.hub.noModelsYet') }}
                  </span>
                </div>

                <span
                  :data-pool-detail="pool.vendor"
                  class="inline-flex items-center gap-1 text-xs font-medium text-muted-foreground transition-colors group-hover:text-studio-violet"
                >
                  {{ t('llmGateway.hub.detailLink') }}
                  <ArrowRight class="size-3.5" aria-hidden="true" />
                </span>
              </div>
            </article>
          </div>
        </div>

        <!-- Recent Accounts Health Table -->
        <section class="studio-panel overflow-hidden">
          <div class="flex flex-wrap items-center justify-between gap-2 border-b border-border/70 px-4 py-3">
            <div>
              <h2 class="text-sm font-bold text-foreground">
                {{ t('llmGateway.hub.recentAccountsTitle') }}
              </h2>
              <p class="text-xs text-muted-foreground">
                {{ t('llmGateway.hub.recentAccountsSubtitle') }}
              </p>
            </div>
            <Button
              variant="outline"
              size="sm"
              class="h-7 px-2.5 text-xs"
              data-nav="accounts"
              @click="router.push({ name: 'llm-gateway-accounts' })"
            >
              {{ t('llmGateway.hub.viewAllBtn') }}
              <ArrowRight class="size-3.5" aria-hidden="true" />
            </Button>
          </div>

          <div class="overflow-x-auto">
            <table class="w-full text-left text-xs">
              <thead class="border-b border-border/60 bg-surface/30 text-[11px] uppercase text-muted-foreground">
                <tr>
                  <th class="px-4 py-2.5 font-semibold">ACCOUNT</th>
                  <th class="px-4 py-2.5 font-semibold">VENDOR</th>
                  <th class="px-4 py-2.5 font-semibold">STATUS</th>
                  <th class="px-4 py-2.5 font-semibold">
                    {{ t('llmGateway.hub.colRemainingQuota') }}
                  </th>
                </tr>
              </thead>
              <tbody class="divide-y divide-border/60">
                <tr
                  v-for="acc in recentAccounts"
                  :key="acc.account_key"
                  :data-account="acc.account_key"
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
                      {{ acc.account_email || acc.account_key }}
                    </div>
                  </td>
                  <td class="px-4 py-2.5 align-middle">
                    {{ vendorName(acc.vendor) }}
                  </td>
                  <td class="px-4 py-2.5 align-middle">
                    <span
                      class="inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] font-medium"
                      :class="statusBadgeClass(acc.status)"
                    >
                      {{ statusLabel(acc) }}
                    </span>
                  </td>
                  <td class="min-w-[130px] px-4 py-2.5 align-middle">
                    <template v-if="accountQuotaPct(acc) !== null">
                      <div class="mb-1 flex items-center justify-between text-[11px]">
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
                    <div
                      v-else-if="loadingQuotas"
                      data-quota-skeleton
                      class="card-shimmer h-3 w-20 rounded bg-muted/50"
                      aria-hidden="true"
                    />
                    <span v-else class="text-[11px] text-muted-foreground">—</span>
                  </td>
                </tr>
                <tr v-if="!recentAccounts.length">
                  <td colspan="4" class="px-4 py-8 text-center text-xs text-muted-foreground">
                    {{ t('llmGateway.empty') }}
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>
      </div>

      <!-- Right Aside Column -->
      <aside class="space-y-3.5">
        <!-- Needs Attention Card -->
        <section class="studio-panel overflow-hidden">
          <div class="flex items-center justify-between gap-2 border-b border-border/70 px-4 py-3">
            <div class="flex items-center gap-2">
              <AlertTriangle class="size-4 text-warning" aria-hidden="true" />
              <h2 class="text-sm font-bold text-foreground">
                {{ t('llmGateway.hub.attentionTitle') }}
              </h2>
            </div>
            <Badge
              variant="outline"
              class="text-[11px]"
              :class="
                issueAccounts.length
                  ? 'border-warning/30 bg-warning/10 text-warning'
                  : 'text-muted-foreground'
              "
            >
              {{ t('llmGateway.hub.attentionBadge', { count: issueAccounts.length }) }}
            </Badge>
          </div>

          <div class="space-y-2.5 p-4">
            <template v-if="issueAccounts.length">
              <div
                v-for="acc in issueAccounts"
                :key="acc.account_key"
                :data-attention-account="acc.account_key"
                role="button"
                tabindex="0"
                class="flex cursor-pointer items-center gap-2.5 rounded-xl border border-border/70 bg-surface/40 p-2.5 transition-colors hover:border-studio-violet/50"
                @click="goAccount(acc.account_key)"
                @keydown.enter.prevent="goAccount(acc.account_key)"
                @keydown.space.prevent="goAccount(acc.account_key)"
              >
                <span
                  class="flex size-8 shrink-0 items-center justify-center rounded-lg border"
                  :class="
                    acc.status === 'reauth_required'
                      ? 'border-danger/30 bg-danger/10 text-danger'
                      : 'border-warning/30 bg-warning/10 text-warning'
                  "
                >
                  <Clock v-if="acc.status === 'cooling_down'" class="size-4" aria-hidden="true" />
                  <KeyRound v-else class="size-4" aria-hidden="true" />
                </span>
                <div class="min-w-0 flex-1">
                  <div class="truncate text-xs font-bold text-foreground">{{ acc.label }}</div>
                  <div class="truncate text-[11px] text-muted-foreground">
                    {{ vendorName(acc.vendor) }} · {{ statusLabel(acc) }}
                  </div>
                </div>
                <Button
                  variant="outline"
                  size="sm"
                  class="h-7 shrink-0 px-2 text-xs"
                  @click.stop="goAccount(acc.account_key)"
                >
                  {{ t('llmGateway.hub.resolveBtn') }}
                  <ChevronRight class="size-3.5" aria-hidden="true" />
                </Button>
              </div>
            </template>
            <p v-else class="text-xs text-muted-foreground">
              {{ t('llmGateway.hub.attentionEmpty') }}
            </p>

            <div class="h-px bg-border/60" />

            <Button
              variant="outline"
              size="sm"
              class="w-full"
              data-nav="quotas"
              @click="router.push({ name: 'llm-gateway-quotas' })"
            >
              <Gauge class="size-4" aria-hidden="true" />
              {{ t('llmGateway.hub.manageQuotasBtn') }}
            </Button>
          </div>
        </section>

        <!-- Quick Access Card -->
        <section class="studio-panel overflow-hidden">
          <div class="border-b border-border/70 px-4 py-3">
            <h2 class="text-sm font-bold text-foreground">
              {{ t('llmGateway.hub.quickAccessTitle') }}
            </h2>
          </div>

          <div class="grid gap-2 p-4">
            <Button
              variant="outline"
              size="sm"
              class="w-full justify-between"
              data-nav="accounts"
              @click="router.push({ name: 'llm-gateway-accounts' })"
            >
              <span class="inline-flex items-center gap-2">
                <User class="size-4 text-studio-violet" aria-hidden="true" />
                {{ t('llmGateway.hub.quickAccounts') }}
              </span>
              <ChevronRight class="size-3.5 text-muted-foreground" aria-hidden="true" />
            </Button>

            <Button
              variant="outline"
              size="sm"
              class="w-full justify-between"
              data-nav="models"
              @click="router.push({ name: 'llm-gateway-models' })"
            >
              <span class="inline-flex items-center gap-2">
                <Cpu class="size-4 text-studio-violet" aria-hidden="true" />
                {{ t('llmGateway.hub.quickModels') }}
              </span>
              <ChevronRight class="size-3.5 text-muted-foreground" aria-hidden="true" />
            </Button>

            <Button
              variant="outline"
              size="sm"
              class="w-full justify-between"
              data-nav="quotas"
              @click="router.push({ name: 'llm-gateway-quotas' })"
            >
              <span class="inline-flex items-center gap-2">
                <Gauge class="size-4 text-studio-violet" aria-hidden="true" />
                {{ t('llmGateway.hub.quickQuotas') }}
              </span>
              <ChevronRight class="size-3.5 text-muted-foreground" aria-hidden="true" />
            </Button>

            <Button
              variant="outline"
              size="sm"
              class="w-full justify-between"
              data-nav="connect"
              @click="router.push({ name: 'llm-gateway-connect' })"
            >
              <span class="inline-flex items-center gap-2">
                <Plus class="size-4 text-studio-violet" aria-hidden="true" />
                {{ t('llmGateway.hub.quickConnect') }}
              </span>
              <ChevronRight class="size-3.5 text-muted-foreground" aria-hidden="true" />
            </Button>
          </div>
        </section>
      </aside>
    </div>
  </section>
</template>

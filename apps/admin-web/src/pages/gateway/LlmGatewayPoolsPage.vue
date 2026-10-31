<script setup lang="ts">
import { ArrowLeft, ArrowRight, Layers, Plus, RefreshCw } from '@lucide/vue'
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { llmGatewayApi } from '@/api/llm-gateway'
import type {
  GatewayAccountView,
  GatewayPoolModelView,
  GatewayVendor,
  GatewayVendorInfo,
} from '@/api/types/llm-gateway'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { vendorMeta } from './llmGatewayUi'

const router = useRouter()
const { t } = useI18n()

const vendors = ref<GatewayVendorInfo[]>([])
const accounts = ref<GatewayAccountView[]>([])
const selectedFilter = ref<'all' | GatewayVendor>('all')
const loading = ref(true)
const errorMessage = ref('')

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

const visiblePoolCards = computed(() =>
  selectedFilter.value === 'all'
    ? poolCards.value
    : poolCards.value.filter((p) => p.vendor === selectedFilter.value),
)

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
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    loading.value = false
  }
}

function goPool(vendor: GatewayVendor) {
  void router.push({ name: 'llm-gateway-pool', params: { vendor } })
}

onMounted(() => {
  void load()
})
</script>

<template>
  <section class="space-y-4" data-testid="llm-gateway-pools-page">
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
          {{ t('llmGateway.poolsPage.eyebrow') }}
        </p>
        <h1 class="text-xl font-bold tracking-tight sm:text-2xl">
          {{ t('llmGateway.poolsPage.title') }}
        </h1>
        <p class="max-w-2xl text-xs text-muted-foreground sm:text-sm">
          {{ t('llmGateway.poolsPage.subtitle') }}
        </p>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <Button variant="outline" size="sm" :disabled="loading" @click="load">
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" aria-hidden="true" />
          {{ t('common.refresh') }}
        </Button>
        <Button size="sm" @click="router.push({ name: 'llm-gateway-connect' })">
          <Plus class="size-4" aria-hidden="true" />
          {{ t('llmGateway.hub.connectCta') }}
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

    <!-- Vendor Filter Pill Tabs -->
    <div class="flex flex-wrap items-center gap-1.5 rounded-xl border border-border/70 bg-surface/40 p-1.5">
      <button
        type="button"
        data-filter="all"
        class="rounded-lg px-2.5 py-1 text-xs font-semibold transition-colors"
        :class="
          selectedFilter === 'all'
            ? 'bg-studio-violet/15 text-studio-violet'
            : 'text-muted-foreground hover:bg-accent hover:text-foreground'
        "
        @click="selectedFilter = 'all'"
      >
        {{ t('common.all') }}
      </button>
      <button
        v-for="v in vendors"
        :key="v.vendor"
        type="button"
        :data-filter="v.vendor"
        class="rounded-lg px-2.5 py-1 text-xs font-semibold transition-colors"
        :class="
          selectedFilter === v.vendor
            ? 'bg-studio-violet/15 text-studio-violet'
            : 'text-muted-foreground hover:bg-accent hover:text-foreground'
        "
        @click="selectedFilter = v.vendor"
      >
        {{ v.display_name }}
      </button>
    </div>

    <div
      v-if="loading"
      class="grid gap-3.5 sm:grid-cols-2 lg:grid-cols-3"
      role="status"
      aria-busy="true"
    >
      <div
        v-for="slot in 3"
        :key="slot"
        class="studio-panel card-shimmer h-40 p-4"
        aria-hidden="true"
      />
    </div>

    <div v-else class="grid gap-3.5 sm:grid-cols-2 lg:grid-cols-3">
      <article
        v-for="pool in visiblePoolCards"
        :key="pool.vendor"
        :data-pool="pool.vendor"
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
                <h2 class="truncate text-sm font-bold text-foreground">
                  {{ pool.displayName }}
                </h2>
                <p class="font-mono text-xs text-muted-foreground">{{ pool.poolBinding }}</p>
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
              <div class="mt-1 text-base font-bold text-foreground">
                {{ pool.models.length }}
              </div>
            </div>
          </div>
        </div>

        <div class="mt-3 flex items-center justify-between gap-2 pt-1">
          <div class="flex flex-wrap items-center gap-1">
            <template v-if="pool.tags.length">
              <span
                v-for="tag in pool.tags.slice(0, 3)"
                :key="tag"
                class="rounded border border-border/70 bg-surface px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
              >
                {{ tag }}
              </span>
              <span
                v-if="pool.tags.length > 3"
                class="rounded border border-border/70 bg-surface px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
              >
                +{{ pool.tags.length - 3 }}
              </span>
            </template>
            <span v-else class="text-xs text-muted-foreground">
              {{ t('llmGateway.hub.noModelsYet') }}
            </span>
          </div>

          <span class="inline-flex items-center gap-1 text-xs font-medium text-muted-foreground transition-colors group-hover:text-studio-violet">
            {{ t('llmGateway.hub.detailLink') }}
            <ArrowRight class="size-3.5" aria-hidden="true" />
          </span>
        </div>
      </article>
    </div>
  </section>
</template>

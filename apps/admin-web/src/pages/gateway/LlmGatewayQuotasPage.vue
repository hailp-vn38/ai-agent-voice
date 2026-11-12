<script setup lang="ts">
import { AlertTriangle, CheckCircle2, Clock, Gauge, RefreshCw } from '@lucide/vue'
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { llmGatewayApi } from '@/api/llm-gateway'
import type {
  GatewayAccountQuota,
  GatewayAccountView,
  GatewayVendor,
  GatewayVendorInfo,
} from '@/api/types/llm-gateway'
import { Badge } from '@/components/ui/badge'
import DetailHero from '@/components/admin/DetailHero.vue'
import { Gauge as HeroIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useErrorToast } from '@/composables/useNotify'
import {
  primaryQuotaPercent,
  quotaBarColorClass,
  secondaryQuotaPercent,
  statusBadgeClass,
} from './llmGatewayUi'

const router = useRouter()
const { t } = useI18n()

const vendors = ref<GatewayVendorInfo[]>([])
const accounts = ref<GatewayAccountView[]>([])
const quotasByAccount = ref<Record<string, GatewayAccountQuota>>({})
const loading = ref(true)
const loadingQuotas = ref(false)
const quotaBusyKey = ref<string | null>(null)
const errorMessage = ref('')
useErrorToast(() => errorMessage.value, { retry: () => load() })

const activeCount = computed(
  () => accounts.value.filter((a) => a.status === 'active').length,
)
const cooldownCount = computed(
  () => accounts.value.filter((a) => a.status === 'cooling_down').length,
)
const reauthCount = computed(
  () => accounts.value.filter((a) => a.status === 'reauth_required').length,
)

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

function statusNote(acc: GatewayAccountView): string {
  switch (acc.status) {
    case 'cooling_down':
      return 'Chờ cooldown'
    case 'reauth_required':
      return 'Reauthenticate'
    case 'disabled':
      return 'Disabled'
    default:
      return 'Normal'
  }
}

// Vendor-reported notice (e.g. account not eligible/verified) that makes requests fail even
// while the account is active and quota windows look healthy.
function quotaMessage(acc: GatewayAccountView): string | null {
  return (quotasByAccount.value[acc.account_key] ?? acc.quota)?.message?.trim() || null
}

function primaryPct(acc: GatewayAccountView): number | null {
  return primaryQuotaPercent(quotasByAccount.value[acc.account_key] ?? acc.quota)
}

function secondaryPct(acc: GatewayAccountView): number | null {
  return secondaryQuotaPercent(quotasByAccount.value[acc.account_key] ?? acc.quota)
}

function primaryWindowName(acc: GatewayAccountView): string {
  const q = quotasByAccount.value[acc.account_key] ?? acc.quota
  return q?.windows[0]?.name || '5h window'
}

function secondaryWindowName(acc: GatewayAccountView): string {
  const q = quotasByAccount.value[acc.account_key] ?? acc.quota
  return q?.windows[1]?.name || 'Weekly'
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
  } catch (cause) {
    if (force) errorMessage.value = formatApiError(cause)
  } finally {
    loadingQuotas.value = false
  }
}

async function refreshSingleQuota(acc: GatewayAccountView) {
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
    quotaBusyKey.value = null
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
  <section class="space-y-4" data-testid="llm-gateway-quotas-page">
    <DetailHero tone="gateway" :eyebrow="t('llmGateway.quotasPage.eyebrow')" :title="t('llmGateway.quotasPage.title')" :description="t('llmGateway.quotasPage.subtitle')" :back-label="t('llmGateway.backToGateway')" @back="router.push({ name: 'llm-gateway' })">
      <template #icon><HeroIcon :size="28" /></template>
      <template #actions>
        <button type="button" class="hero-primary"
          data-action="refresh-quotas"
          :disabled="loadingQuotas || loading"
          @click="fetchAllQuotas(true)"
        >
          <RefreshCw class="size-4" :class="{ 'animate-spin': loadingQuotas }" aria-hidden="true" />
          {{ t('llmGateway.refreshQuotas') }}
        </button>
      </template>
    </DetailHero>

    <!-- 4 KPI Cards -->
    <div class="grid grid-cols-2 gap-3 lg:grid-cols-4">
      <div class="studio-panel flex items-center gap-3 p-3.5">
        <span class="inline-flex size-10 shrink-0 items-center justify-center rounded-xl border border-studio-violet/25 bg-studio-violet/10 text-studio-violet">
          <Gauge class="size-4" aria-hidden="true" />
        </span>
        <div>
          <div class="text-xl font-extrabold tracking-tight text-foreground">
            {{ accounts.length }}
          </div>
          <div class="text-xs text-muted-foreground">Accounts</div>
        </div>
      </div>

      <div class="studio-panel flex items-center gap-3 p-3.5">
        <span class="inline-flex size-10 shrink-0 items-center justify-center rounded-xl border border-success/25 bg-success/10 text-success">
          <CheckCircle2 class="size-4" aria-hidden="true" />
        </span>
        <div>
          <div class="text-xl font-extrabold tracking-tight text-foreground">
            {{ activeCount }}
          </div>
          <div class="text-xs text-muted-foreground">Healthy</div>
        </div>
      </div>

      <div class="studio-panel flex items-center gap-3 p-3.5">
        <span class="inline-flex size-10 shrink-0 items-center justify-center rounded-xl border border-warning/25 bg-warning/10 text-warning">
          <Clock class="size-4" aria-hidden="true" />
        </span>
        <div>
          <div class="text-xl font-extrabold tracking-tight text-foreground">
            {{ cooldownCount }}
          </div>
          <div class="text-xs text-muted-foreground">Cooldown</div>
        </div>
      </div>

      <div class="studio-panel flex items-center gap-3 p-3.5">
        <span class="inline-flex size-10 shrink-0 items-center justify-center rounded-xl border border-danger/25 bg-danger/10 text-danger">
          <AlertTriangle class="size-4" aria-hidden="true" />
        </span>
        <div>
          <div class="text-xl font-extrabold tracking-tight text-foreground">
            {{ reauthCount }}
          </div>
          <div class="text-xs text-muted-foreground">Re-auth required</div>
        </div>
      </div>
    </div>

    <!-- Account Quota Matrix -->
    <div class="studio-panel overflow-hidden">
      <div class="flex flex-wrap items-center justify-between gap-2 border-b border-border/70 px-4 py-3">
        <div>
          <h2 class="text-sm font-bold text-foreground">
            {{ t('llmGateway.quotasPage.matrixTitle') }}
          </h2>
          <p class="text-xs text-muted-foreground">
            {{ t('llmGateway.quotasPage.matrixSubtitle') }}
          </p>
        </div>
        <Badge variant="outline" class="text-xs">{{ accounts.length }} accounts</Badge>
      </div>

      <div class="overflow-x-auto">
        <table class="w-full text-left text-xs">
          <thead class="border-b border-border/60 bg-surface/30 text-[11px] uppercase text-muted-foreground">
            <tr>
              <th class="px-4 py-2.5 font-semibold">ACCOUNT</th>
              <th class="px-4 py-2.5 font-semibold">TRẠNG THÁI</th>
              <th class="px-4 py-2.5 font-semibold">5H REMAINING</th>
              <th class="px-4 py-2.5 font-semibold">WEEKLY REMAINING</th>
              <th class="px-4 py-2.5 font-semibold">GHI CHÚ</th>
              <th class="px-4 py-2.5 text-right font-semibold" />
            </tr>
          </thead>
          <tbody class="divide-y divide-border/60">
            <tr
              v-for="acc in accounts"
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
                  {{ vendorName(acc.vendor) }}
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
              <td class="min-w-[160px] px-4 py-2.5 align-middle">
                <template v-if="primaryPct(acc) !== null">
                  <div class="mb-1 flex items-center justify-between text-[11px]">
                    <span class="text-muted-foreground">{{ primaryWindowName(acc) }}</span>
                    <strong class="font-mono">{{ primaryPct(acc) }}%</strong>
                  </div>
                  <div class="h-1.5 w-full overflow-hidden rounded-full bg-muted">
                    <div
                      class="h-full rounded-full"
                      :class="quotaBarColorClass(primaryPct(acc) ?? 100, false)"
                      :style="{ width: `${primaryPct(acc)}%` }"
                    />
                  </div>
                </template>
                <span v-else class="text-muted-foreground">—</span>
              </td>
              <td class="min-w-[150px] px-4 py-2.5 align-middle">
                <template v-if="secondaryPct(acc) !== null">
                  <div class="mb-1 flex items-center justify-between text-[11px]">
                    <span class="text-muted-foreground">{{ secondaryWindowName(acc) }}</span>
                    <strong class="font-mono">{{ secondaryPct(acc) }}%</strong>
                  </div>
                  <div class="h-1.5 w-full overflow-hidden rounded-full bg-muted">
                    <div
                      class="h-full rounded-full"
                      :class="quotaBarColorClass(secondaryPct(acc) ?? 100, false)"
                      :style="{ width: `${secondaryPct(acc)}%` }"
                    />
                  </div>
                </template>
                <span v-else class="text-muted-foreground">—</span>
              </td>
              <td class="max-w-xs px-4 py-2.5 align-middle text-muted-foreground">
                <span
                  v-if="quotaMessage(acc)"
                  role="alert"
                  class="line-clamp-2 break-words text-warning"
                  :title="quotaMessage(acc) ?? undefined"
                >
                  {{ quotaMessage(acc) }}
                </span>
                <template v-else>{{ statusNote(acc) }}</template>
              </td>
              <td class="px-4 py-2.5 text-right align-middle" @click.stop>
                <Button
                  variant="outline"
                  size="sm"
                  class="h-7 px-2 text-xs"
                  :disabled="quotaBusyKey === acc.account_key || loadingQuotas"
                  @click="refreshSingleQuota(acc)"
                >
                  <RefreshCw
                    class="size-3.5"
                    :class="{ 'animate-spin': quotaBusyKey === acc.account_key }"
                    aria-hidden="true"
                  />
                </Button>
              </td>
            </tr>
            <tr v-if="!loading && !accounts.length">
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

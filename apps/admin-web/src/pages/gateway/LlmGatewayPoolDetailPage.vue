<script setup lang="ts">
import { ArrowRight, ChevronRight, Cpu, Info, KeyRound, Plus, Trash2, User } from '@lucide/vue'
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { llmGatewayApi } from '@/api/llm-gateway'
import type {
  GatewayAccountQuota,
  GatewayAccountView,
  GatewayPoolDetailView,
  GatewayPoolModelView,
  GatewayVendor,
} from '@/api/types/llm-gateway'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import DetailHero from '@/components/admin/DetailHero.vue'
import { Layers as HeroIcon } from '@lucide/vue'
import { useI18n } from '@/composables/useI18n'
import { useErrorToast } from '@/composables/useNotify'
import {
  loginModeLabel,
  primaryQuotaPercent,
  quotaBarColorClass,
  statusBadgeClass,
  vendorMeta,
} from './llmGatewayUi'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()

const vendorParam = computed(
  () => (String(route.params.vendor || 'openai_codex') as GatewayVendor),
)

const poolDetail = ref<GatewayPoolDetailView | null>(null)
const quotasByAccount = ref<Record<string, GatewayAccountQuota>>({})
const activeTab = ref<'accounts' | 'models' | 'presets' | 'config'>('accounts')
const loading = ref(true)
const busyKey = ref<string | null>(null)
const errorMessage = ref('')
useErrorToast(() => errorMessage.value, { retry: () => load() })

const deleteModelOpen = ref(false)
const deletingModel = ref<GatewayPoolModelView | null>(null)

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

async function load() {
  loading.value = true
  errorMessage.value = ''
  try {
    const [detailRes, quotaRes] = await Promise.all([
      llmGatewayApi.getPoolDetail(vendorParam.value),
      llmGatewayApi.listQuotas({ vendor: vendorParam.value }).catch(() => ({ quotas: [] })),
    ])
    poolDetail.value = detailRes.pool
    const map: Record<string, GatewayAccountQuota> = {}
    for (const q of quotaRes.quotas) {
      map[q.account_key] = q
    }
    quotasByAccount.value = map
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    loading.value = false
  }
}

async function toggleModelEnabled(m: GatewayPoolModelView) {
  if (!poolDetail.value) return
  busyKey.value = m.provider_key
  errorMessage.value = ''
  try {
    await llmGatewayApi.updatePoolModel(poolDetail.value.vendor, m.provider_key, {
      enabled: !m.enabled,
    })
    await load()
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    busyKey.value = null
  }
}

function promptDeleteModel(m: GatewayPoolModelView) {
  deletingModel.value = m
  deleteModelOpen.value = true
}

async function confirmDeleteModel() {
  if (!deletingModel.value || !poolDetail.value) return
  const target = deletingModel.value
  busyKey.value = target.provider_key
  errorMessage.value = ''
  try {
    await llmGatewayApi.deletePoolModel(poolDetail.value.vendor, target.provider_key)
    deleteModelOpen.value = false
    deletingModel.value = null
    await load()
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    busyKey.value = null
  }
}

function goAccount(accountKey: string) {
  void router.push({ name: 'llm-gateway-account', params: { accountKey } })
}

function goEditModel(providerKey: string) {
  void router.push({
    name: 'llm-gateway-model',
    params: { providerKey },
    query: { vendor: vendorParam.value },
  })
}

function goCreateModel(preset?: string) {
  void router.push({
    name: 'llm-gateway-model-create',
    query: preset
      ? { vendor: vendorParam.value, preset }
      : { vendor: vendorParam.value },
  })
}

function goConnectVendor() {
  void router.push({
    name: 'llm-gateway-connect',
    query: { vendor: vendorParam.value },
  })
}

watch(vendorParam, () => {
  void load()
})

onMounted(() => {
  const initialTab = String(route.query.tab || '')
  if (
    initialTab === 'accounts' ||
    initialTab === 'models' ||
    initialTab === 'presets' ||
    initialTab === 'config'
  ) {
    activeTab.value = initialTab
  }
  void load()
})
</script>

<template>
  <section class="space-y-4" data-testid="llm-gateway-pool-detail-page">
    <DetailHero tone="gateway" :eyebrow="t('llmGateway.poolPage.eyebrow')" :title="poolDetail?.display_name || vendorParam" :description="t('llmGateway.poolPage.subtitle', { name: poolDetail?.display_name || vendorParam, })" :back-label="t('llmGateway.backToPools')" @back="router.push({ name: 'llm-gateway-pools' })">
      <template #icon><HeroIcon :size="28" /></template>
      <template #actions>
        <button type="button" class="hero-ghost" data-action="create-model" @click="goCreateModel()">
          <Plus class="size-4" aria-hidden="true" />
          {{ t('llmGateway.pool.addModelBtn') }}
        </button>
        <button type="button" class="hero-primary" data-action="connect-vendor" @click="goConnectVendor()">
          <KeyRound class="size-4" aria-hidden="true" />
          {{ t('llmGateway.addToPool') }}
        </button>
      </template>
    </DetailHero>

    <div
      v-if="loading && !poolDetail"
      data-pool-detail-skeleton
      role="status"
      aria-busy="true"
      class="space-y-3.5"
    >
      <div class="grid gap-3 sm:grid-cols-3">
        <div
          v-for="slot in 3"
          :key="slot"
          class="studio-panel card-shimmer h-20 p-3.5"
          aria-hidden="true"
        />
      </div>
      <div class="studio-panel card-shimmer h-64 p-4" aria-hidden="true" />
    </div>

    <template v-else-if="poolDetail">
      <!-- 3 KPI Mini Cards -->
      <div class="grid gap-3 sm:grid-cols-3" data-testid="pool-detail-content">
        <div class="studio-panel flex items-center gap-3 p-3.5">
          <span
            class="inline-flex size-10 shrink-0 items-center justify-center rounded-xl border font-mono text-xs font-extrabold"
            :class="vendorMeta(poolDetail.vendor).logoClass"
          >
            {{ vendorMeta(poolDetail.vendor).short }}
          </span>
          <div class="min-w-0">
            <div class="text-xs text-muted-foreground">Pool Binding</div>
            <div class="truncate font-mono text-sm font-bold text-foreground">
              {{ poolDetail.pool_binding }}
            </div>
            <div class="truncate font-mono text-[11px] text-muted-foreground">
              {{ poolDetail.default_base_url }}
            </div>
          </div>
        </div>

        <div class="studio-panel flex items-center gap-3 p-3.5">
          <span class="inline-flex size-10 shrink-0 items-center justify-center rounded-xl border border-studio-violet/25 bg-studio-violet/10 text-studio-violet">
            <User class="size-4" aria-hidden="true" />
          </span>
          <div>
            <div class="text-lg font-extrabold tracking-tight text-foreground">
              {{ poolDetail.active_accounts }}/{{ poolDetail.total_accounts }}
            </div>
            <div class="text-xs text-muted-foreground">Active accounts</div>
          </div>
        </div>

        <div class="studio-panel flex items-center gap-3 p-3.5">
          <span class="inline-flex size-10 shrink-0 items-center justify-center rounded-xl border border-studio-violet/25 bg-studio-violet/10 text-studio-violet">
            <Cpu class="size-4" aria-hidden="true" />
          </span>
          <div>
            <div class="text-lg font-extrabold tracking-tight text-foreground">
              {{ poolDetail.models.length }}
            </div>
            <div class="text-xs text-muted-foreground">Registered models</div>
          </div>
        </div>
      </div>

      <!-- Main Card with 4 Pill Tabs -->
      <div class="studio-panel overflow-hidden">
        <div class="flex flex-wrap items-center justify-between gap-2 border-b border-border/70 px-4 py-3">
          <div class="flex flex-wrap items-center gap-1 rounded-xl border border-border/70 bg-surface/50 p-1">
            <button
              type="button"
              data-pool-tab="accounts"
              class="rounded-lg px-2.5 py-1 text-xs font-semibold transition-colors"
              :class="
                activeTab === 'accounts'
                  ? 'bg-studio-violet/15 text-studio-violet'
                  : 'text-muted-foreground hover:text-foreground'
              "
              @click="activeTab = 'accounts'"
            >
              {{ t('llmGateway.poolPage.tabAccounts') }} ({{ poolDetail.accounts.length }})
            </button>
            <button
              type="button"
              data-pool-tab="models"
              class="rounded-lg px-2.5 py-1 text-xs font-semibold transition-colors"
              :class="
                activeTab === 'models'
                  ? 'bg-studio-violet/15 text-studio-violet'
                  : 'text-muted-foreground hover:text-foreground'
              "
              @click="activeTab = 'models'"
            >
              {{ t('llmGateway.poolPage.tabModels') }} ({{ poolDetail.models.length }})
            </button>
            <button
              type="button"
              data-pool-tab="presets"
              class="rounded-lg px-2.5 py-1 text-xs font-semibold transition-colors"
              :class="
                activeTab === 'presets'
                  ? 'bg-studio-violet/15 text-studio-violet'
                  : 'text-muted-foreground hover:text-foreground'
              "
              @click="activeTab = 'presets'"
            >
              {{ t('llmGateway.poolPage.tabPresets') }}
            </button>
            <button
              type="button"
              data-pool-tab="config"
              class="rounded-lg px-2.5 py-1 text-xs font-semibold transition-colors"
              :class="
                activeTab === 'config'
                  ? 'bg-studio-violet/15 text-studio-violet'
                  : 'text-muted-foreground hover:text-foreground'
              "
              @click="activeTab = 'config'"
            >
              {{ t('llmGateway.poolPage.tabConfig') }}
            </button>
          </div>

          <div>
            <Button
              v-if="activeTab === 'accounts'"
              variant="outline"
              size="sm"
              class="h-7 px-2.5 text-xs"
              @click="goConnectVendor()"
            >
              <Plus class="size-3.5" aria-hidden="true" />
              {{ t('llmGateway.hub.connectCta') }}
            </Button>
            <Button
              v-else-if="activeTab === 'models'"
              variant="outline"
              size="sm"
              class="h-7 px-2.5 text-xs"
              @click="goCreateModel()"
            >
              <Plus class="size-3.5" aria-hidden="true" />
              {{ t('llmGateway.pool.addModelBtn') }}
            </Button>
          </div>
        </div>

        <!-- TAB 1: OAuth Accounts -->
        <div v-if="activeTab === 'accounts'" data-testid="pool-detail-accounts" class="overflow-x-auto">
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
                v-for="acc in poolDetail.accounts"
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
                    {{ poolDetail.display_name }} · {{ acc.account_email || acc.account_key }}
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
                <td class="min-w-[130px] px-4 py-2.5 align-middle">
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
              <tr v-if="!poolDetail.accounts.length">
                <td colspan="6" class="px-4 py-8 text-center text-xs text-muted-foreground">
                  {{ t('llmGateway.empty') }}
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <!-- TAB 2: Pool Models -->
        <div v-else-if="activeTab === 'models'" class="overflow-x-auto">
          <table class="w-full text-left text-xs">
            <thead class="border-b border-border/60 bg-surface/30 text-[11px] uppercase text-muted-foreground">
              <tr>
                <th class="px-4 py-2.5 font-semibold">MODEL</th>
                <th class="px-4 py-2.5 font-semibold">BINDING</th>
                <th class="px-4 py-2.5 font-semibold">CAPABILITY TAGS</th>
                <th class="px-4 py-2.5 font-semibold">STATUS</th>
                <th class="px-4 py-2.5 font-semibold">USED BY</th>
                <th class="px-4 py-2.5 text-right font-semibold" />
              </tr>
            </thead>
            <tbody class="divide-y divide-border/60">
              <tr
                v-for="m in poolDetail.models"
                :key="m.provider_key"
                :data-model="m.provider_key"
                :data-pool-model-row="m.provider_key"
                role="button"
                tabindex="0"
                class="cursor-pointer transition-colors hover:bg-accent/25"
                @click="goEditModel(m.provider_key)"
                @keydown.enter.prevent="goEditModel(m.provider_key)"
                @keydown.space.prevent="goEditModel(m.provider_key)"
              >
                <td class="px-4 py-2.5 align-middle">
                  <div class="font-bold text-foreground">{{ m.display_name }}</div>
                  <div class="font-mono text-[11px] text-muted-foreground">{{ m.model }}</div>
                </td>
                <td class="px-4 py-2.5 align-middle font-mono text-[11px]">
                  {{ m.gateway_binding }}
                </td>
                <td class="px-4 py-2.5 align-middle">
                  <div class="flex flex-wrap gap-1">
                    <span
                      v-for="tag in m.tags.slice(0, 3)"
                      :key="tag"
                      class="rounded border border-studio-violet/30 bg-studio-violet/10 px-1.5 py-0.5 font-mono text-[10px] text-studio-violet"
                    >
                      {{ tag }}
                    </span>
                    <span
                      v-if="m.tags.length > 3"
                      class="rounded border border-border/70 bg-surface px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
                    >
                      +{{ m.tags.length - 3 }}
                    </span>
                  </div>
                </td>
                <td class="px-4 py-2.5 align-middle">
                  <span
                    class="inline-flex items-center rounded-full border px-2 py-0.5 text-[11px] font-medium"
                    :class="
                      m.enabled
                        ? 'border-success/30 bg-success/10 text-success'
                        : 'border-border bg-muted/50 text-muted-foreground'
                    "
                  >
                    {{ m.enabled ? 'Enabled' : 'Disabled' }}
                  </span>
                </td>
                <td class="px-4 py-2.5 align-middle">
                  {{ m.template_usage_count }} templates
                </td>
                <td class="px-4 py-2.5 text-right align-middle">
                  <div class="inline-flex items-center justify-end gap-1" @click.stop>
                    <Button
                      variant="outline"
                      size="sm"
                      class="h-7 px-2 text-xs"
                      :data-edit-pool-model="m.provider_key"
                      @click="goEditModel(m.provider_key)"
                    >
                      Cấu hình
                      <ChevronRight class="size-3.5" aria-hidden="true" />
                    </Button>
                    <Button
                      variant="outline"
                      size="sm"
                      class="h-7 px-2 text-xs"
                      :data-toggle-pool-model="m.provider_key"
                      :disabled="busyKey === m.provider_key"
                      @click="toggleModelEnabled(m)"
                    >
                      {{ m.enabled ? t('llmGateway.disable') : t('llmGateway.enable') }}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      class="h-7 px-2 text-xs text-danger"
                      :data-delete-pool-model="m.provider_key"
                      :disabled="busyKey === m.provider_key || m.template_usage_count > 0"
                      @click="promptDeleteModel(m)"
                    >
                      <Trash2 class="size-3.5" aria-hidden="true" />
                    </Button>
                  </div>
                </td>
              </tr>
              <tr v-if="!poolDetail.models.length">
                <td colspan="6" class="px-4 py-8 text-center text-xs text-muted-foreground">
                  {{ t('llmGateway.pool.noModels') }}
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <!-- TAB 3: Model Presets -->
        <div v-else-if="activeTab === 'presets'" class="space-y-3.5 p-4">
          <div class="flex items-start gap-2.5 rounded-xl border border-studio-violet/25 bg-studio-violet/5 p-3 text-xs">
            <Info class="mt-0.5 size-4 shrink-0 text-studio-violet" aria-hidden="true" />
            <div>
              <div class="font-bold text-foreground">{{ t('llmGateway.poolPage.tabPresets') }}</div>
              <p class="mt-0.5 text-muted-foreground">
                {{ t('llmGateway.pool.presetsHint') }}
              </p>
            </div>
          </div>

          <div
            v-if="poolDetail.preset_models.length"
            class="grid gap-3 sm:grid-cols-2"
          >
            <article
              v-for="preset in poolDetail.preset_models"
              :key="preset.model"
              :data-preset-model="preset.model"
              role="button"
              tabindex="0"
              class="flex cursor-pointer flex-col justify-between rounded-xl border border-border/70 bg-surface/40 p-3.5 transition-colors hover:border-studio-violet/50"
              @click="goCreateModel(preset.model)"
              @keydown.enter.prevent="goCreateModel(preset.model)"
              @keydown.space.prevent="goCreateModel(preset.model)"
            >
              <div>
                <div class="flex items-start justify-between gap-2">
                  <div>
                    <div class="text-sm font-bold text-foreground">{{ preset.display_name }}</div>
                    <div class="font-mono text-xs text-muted-foreground">{{ preset.model }}</div>
                  </div>
                  <Badge variant="outline" class="text-[10px]">Preset</Badge>
                </div>

                <div class="my-2.5 flex flex-wrap gap-1">
                  <span
                    v-for="tag in preset.tags"
                    :key="tag"
                    class="rounded border border-studio-violet/30 bg-studio-violet/10 px-1.5 py-0.5 font-mono text-[10px] text-studio-violet"
                  >
                    {{ tag }}
                  </span>
                </div>
              </div>

              <div class="pt-1">
                <Button
                  variant="outline"
                  size="sm"
                  class="h-7 px-2.5 text-xs"
                  @click.stop="goCreateModel(preset.model)"
                >
                  {{ t('llmGateway.poolPage.usePresetBtn') }}
                  <ArrowRight class="size-3.5" aria-hidden="true" />
                </Button>
              </div>
            </article>
          </div>
          <p v-else class="py-6 text-center text-xs text-muted-foreground">
            {{ t('llmGateway.pool.noModels') }}
          </p>
        </div>

        <!-- TAB 4: Pool Info -->
        <div v-else class="grid gap-4 p-4 sm:grid-cols-2">
          <div>
            <h3 class="mb-2.5 text-sm font-bold text-foreground">Endpoint & Binding</h3>
            <dl class="divide-y divide-border/60 text-xs">
              <div class="flex items-center justify-between py-2">
                <dt class="text-muted-foreground">Vendor ID</dt>
                <dd class="font-mono font-medium text-foreground">{{ poolDetail.vendor }}</dd>
              </div>
              <div class="flex items-center justify-between py-2">
                <dt class="text-muted-foreground">Binding</dt>
                <dd class="font-mono font-medium text-foreground">{{ poolDetail.pool_binding }}</dd>
              </div>
              <div class="flex items-center justify-between py-2">
                <dt class="text-muted-foreground">OAuth Mode</dt>
                <dd class="font-medium text-foreground">
                  {{ loginModeLabel(poolDetail.login_mode) }}
                </dd>
              </div>
              <div class="flex items-center justify-between gap-2 py-2">
                <dt class="shrink-0 text-muted-foreground">Default Base URL</dt>
                <dd class="truncate font-mono text-foreground">{{ poolDetail.default_base_url }}</dd>
              </div>
              <div class="flex items-center justify-between py-2">
                <dt class="text-muted-foreground">Provider count</dt>
                <dd class="font-medium text-foreground">{{ poolDetail.models.length }}</dd>
              </div>
            </dl>
          </div>

          <div>
            <h3 class="mb-2.5 text-sm font-bold text-foreground">
              {{ t('llmGateway.poolPage.conceptTitle') }}
            </h3>
            <p class="text-xs leading-relaxed text-muted-foreground">
              {{ t('llmGateway.poolPage.conceptDesc') }}
            </p>
          </div>
        </div>
      </div>
    </template>

    <ConfirmDialog
      v-model="deleteModelOpen"
      :title="t('llmGateway.modelDelete.title')"
      :description="
        deletingModel
          ? `${deletingModel.display_name} (${deletingModel.model} · ${deletingModel.provider_key})`
          : ''
      "
      :confirm-label="t('llmGateway.modelDelete.confirm')"
      destructive
      @confirm="confirmDeleteModel"
    />
  </section>
</template>

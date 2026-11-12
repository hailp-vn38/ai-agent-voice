<script setup lang="ts">
import { ArrowRight, Clock, KeyRound, Pencil, RefreshCw, Shield, Trash2 } from '@lucide/vue'
import { computed, onMounted, ref, watch } from 'vue'
import { RouterLink, useRoute, useRouter } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { llmGatewayApi } from '@/api/llm-gateway'
import type {
  GatewayAccountQuota,
  GatewayAccountView,
  GatewayVendorInfo,
} from '@/api/types/llm-gateway'
import BaseModal from '@/components/admin/BaseModal.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import DetailHero from '@/components/admin/DetailHero.vue'
import { User as HeroIcon } from '@lucide/vue'
import { useI18n } from '@/composables/useI18n'
import { useErrorToast } from '@/composables/useNotify'
import {
  formatResetCountdown,
  formatTimestamp,
  formatWindowUsage,
  loginModeLabel,
  maskedCredentialSummary,
  quotaBarColorClass,
  statusBadgeClass,
  vendorMeta,
} from './llmGatewayUi'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()

const accountKeyParam = computed(() => String(route.params.accountKey || ''))

const vendors = ref<GatewayVendorInfo[]>([])
const accounts = ref<GatewayAccountView[]>([])
const quota = ref<GatewayAccountQuota | null>(null)
const loading = ref(true)
const loadingQuota = ref(false)
const busy = ref(false)
const errorMessage = ref('')
useErrorToast(() => errorMessage.value, { retry: () => load() })

const editOpen = ref(false)
const editLabel = ref('')
const editPriority = ref(100)
const deleteOpen = ref(false)

const account = computed(
  () => accounts.value.find((a) => a.account_key === accountKeyParam.value) ?? null,
)

const vendorInfo = computed(() =>
  account.value
    ? vendors.value.find((v) => v.vendor === account.value!.vendor) ?? null
    : null,
)

const effectiveQuota = computed(() => quota.value ?? account.value?.quota ?? null)

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

async function fetchQuota(force = false) {
  if (!accountKeyParam.value) return
  loadingQuota.value = true
  try {
    const res = await llmGatewayApi.getAccountQuota(accountKeyParam.value, force)
    quota.value = res.quota
  } catch {
    /* Non-blocking */
  } finally {
    loadingQuota.value = false
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
    if (account.value) {
      void fetchQuota(false)
    }
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    loading.value = false
  }
}

async function refreshAccount() {
  if (!account.value) return
  busy.value = true
  errorMessage.value = ''
  try {
    const res = await llmGatewayApi.refreshAccount(account.value.account_key)
    accounts.value = accounts.value.map((a) =>
      a.account_key === res.account.account_key ? res.account : a,
    )
    void fetchQuota(true)
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}

async function clearCooldown() {
  if (!account.value) return
  busy.value = true
  errorMessage.value = ''
  try {
    const res = await llmGatewayApi.clearCooldown(account.value.account_key)
    accounts.value = accounts.value.map((a) =>
      a.account_key === res.account.account_key ? res.account : a,
    )
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}

async function toggleEnabled() {
  if (!account.value) return
  busy.value = true
  errorMessage.value = ''
  try {
    const res = await llmGatewayApi.updateAccount(account.value.account_key, {
      enabled: !account.value.enabled,
    })
    accounts.value = accounts.value.map((a) =>
      a.account_key === res.account.account_key ? res.account : a,
    )
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}

function openEditModal() {
  if (!account.value) return
  editLabel.value = account.value.label
  editPriority.value = account.value.priority
  editOpen.value = true
}

async function saveAccountEdit() {
  if (!account.value) return
  const trimmed = editLabel.value.trim()
  if (!trimmed) {
    errorMessage.value = t('llmGateway.editModal.labelRequired')
    return
  }
  busy.value = true
  errorMessage.value = ''
  try {
    const res = await llmGatewayApi.updateAccount(account.value.account_key, {
      label: trimmed,
      priority: Number(editPriority.value) || 0,
    })
    accounts.value = accounts.value.map((a) =>
      a.account_key === res.account.account_key ? res.account : a,
    )
    editOpen.value = false
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}

async function confirmDeleteAccount() {
  if (!account.value) return
  busy.value = true
  errorMessage.value = ''
  try {
    await llmGatewayApi.deleteAccount(account.value.account_key)
    deleteOpen.value = false
    await router.push({ name: 'llm-gateway-accounts' })
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}

watch(accountKeyParam, () => {
  void load()
})

onMounted(() => {
  void load()
})
</script>

<template>
  <section class="space-y-4" data-testid="llm-gateway-account-detail-page">
    <div v-if="loading" class="studio-panel card-shimmer h-56 p-4" aria-hidden="true" />

    <div
      v-else-if="!account"
      class="studio-panel space-y-3 p-8 text-center text-sm text-muted-foreground"
    >
      <p>{{ t('llmGateway.empty') }}</p>
      <Button variant="outline" size="sm" @click="router.push({ name: 'llm-gateway-accounts' })">
        {{ t('llmGateway.backToAccounts') }}
      </Button>
    </div>

    <template v-else>
      <DetailHero tone="gateway" :eyebrow="t('llmGateway.accountPage.eyebrow')" :title="account.label" :description="[vendorInfo?.display_name || account.vendor, account.account_email || account.account_key, 'OAuth Account'].join(' · ')" :back-label="t('llmGateway.backToAccounts')" @back="router.push({ name: 'llm-gateway-accounts' })">
        <template #icon><HeroIcon :size="28" /></template>
        <template #actions>
          <button type="button" class="hero-ghost"
            data-action="account-refresh"
            :disabled="busy"
            @click="refreshAccount"
          >
            <RefreshCw class="size-4" :class="{ 'animate-spin': busy }" aria-hidden="true" />
            {{ t('llmGateway.refreshToken') }}
          </button>
          <button type="button" class="hero-ghost"
            v-if="account.status === 'cooling_down'"
            data-action="clear-cooldown"
            :disabled="busy"
            @click="clearCooldown"
          >
            <Clock class="size-4" aria-hidden="true" />
            {{ t('llmGateway.clearCooldown') }}
          </button>
          <button type="button" class="hero-primary"
            v-if="account.status === 'reauth_required'"
            data-action="reauth"
            @click="
              router.push({
                name: 'llm-gateway-connect',
                query: { vendor: account.vendor, label: account.label },
              })
            "
          >
            <KeyRound class="size-4" aria-hidden="true" />
            {{ t('llmGateway.accountPage.reauthBtn') }}
          </button>
          <button type="button" class="hero-primary" v-else data-action="account-edit" @click="openEditModal">
            <Pencil class="size-4" aria-hidden="true" />
            {{ t('common.edit') }}
          </button>
        </template>
      </DetailHero>

      <div class="grid gap-4 lg:grid-cols-[minmax(0,1fr)_315px]">
        <div class="space-y-3.5">
          <!-- Account Status & Quotas -->
          <section class="studio-panel overflow-hidden">
            <div class="flex flex-wrap items-center justify-between gap-2 border-b border-border/70 px-4 py-3">
              <div class="flex items-center gap-2.5">
                <span
                  class="inline-flex size-9 shrink-0 items-center justify-center rounded-xl border font-mono text-xs font-extrabold"
                  :class="vendorMeta(account.vendor).logoClass"
                >
                  {{ vendorMeta(account.vendor).short }}
                </span>
                <div>
                  <h2 class="text-sm font-bold text-foreground">
                    {{ t('llmGateway.accountPage.statusCardTitle') }}
                  </h2>
                  <p class="text-xs text-muted-foreground">
                    {{ vendorInfo?.display_name || account.vendor }} ·
                    {{ loginModeLabel(vendorInfo?.login_mode) }}
                  </p>
                </div>
              </div>

              <span
                class="inline-flex items-center gap-1 rounded-full border px-2.5 py-0.5 text-xs font-medium"
                :class="statusBadgeClass(account.status)"
              >
                {{ statusLabel(account) }}
              </span>
            </div>

            <div class="space-y-4 p-4">
              <div class="grid gap-3 sm:grid-cols-3">
                <div class="rounded-lg border border-border/60 bg-surface/40 p-3">
                  <div class="text-xs text-muted-foreground">{{ t('llmGateway.col.priority') }}</div>
                  <div class="mt-1 text-lg font-bold text-foreground">{{ account.priority }}</div>
                </div>
                <div class="rounded-lg border border-border/60 bg-surface/40 p-3">
                  <div class="text-xs text-muted-foreground">Model bindings</div>
                  <div class="mt-1 text-lg font-bold text-foreground">
                    {{ account.bound_providers.length }}
                  </div>
                </div>
                <div class="rounded-lg border border-border/60 bg-surface/40 p-3">
                  <div class="text-xs text-muted-foreground">Plan</div>
                  <div class="mt-1 text-lg font-bold text-foreground">
                    {{ effectiveQuota?.plan || 'Standard' }}
                  </div>
                </div>
              </div>

              <div class="h-px bg-border/60" />

              <div>
                <div class="mb-2.5 flex items-center justify-between gap-2">
                  <h3 class="text-sm font-bold text-foreground">
                    {{ t('llmGateway.accountPage.quotaWindowsTitle') }}
                  </h3>
                  <Button
                    variant="outline"
                    size="sm"
                    class="h-7 px-2.5 text-xs"
                    :disabled="loadingQuota"
                    @click="fetchQuota(true)"
                  >
                    <RefreshCw
                      class="size-3.5"
                      :class="{ 'animate-spin': loadingQuota }"
                      aria-hidden="true"
                    />
                    {{ t('llmGateway.quota.checkBtn') }}
                  </Button>
                </div>

                <div
                  v-if="effectiveQuota?.windows?.length"
                  class="grid gap-3 sm:grid-cols-2"
                >
                  <div
                    v-for="w in effectiveQuota.windows"
                    :key="w.name"
                    class="rounded-xl border border-border/70 bg-surface/35 p-3"
                  >
                    <div class="flex items-center justify-between gap-2 text-xs">
                      <span class="text-muted-foreground">{{ w.name }}</span>
                      <strong class="font-mono font-bold text-foreground">
                        {{ formatWindowUsage(w, t('llmGateway.quota.unlimited')) }}
                      </strong>
                    </div>
                    <div class="my-2 h-1.5 w-full overflow-hidden rounded-full bg-muted">
                      <div
                        class="h-full rounded-full"
                        :class="quotaBarColorClass(w.remaining_percentage, w.unlimited)"
                        :style="{
                          width: `${w.unlimited ? 100 : Math.max(0, Math.min(100, w.remaining_percentage))}%`,
                        }"
                      />
                    </div>
                    <p
                      v-if="formatResetCountdown(w.reset_at)"
                      class="text-[11px] text-muted-foreground"
                    >
                      {{
                        t('llmGateway.quota.resetsIn', {
                          time: formatResetCountdown(w.reset_at),
                        })
                      }}
                    </p>
                  </div>
                </div>
                <p v-else class="text-xs text-muted-foreground">
                  {{ effectiveQuota?.message || t('llmGateway.quota.noData') }}
                </p>
                <p
                  v-if="effectiveQuota?.windows?.length && effectiveQuota.message"
                  role="alert"
                  data-testid="gateway-account-quota-message"
                  class="mt-3 break-words rounded-xl border border-warning/40 bg-warning/10 p-3 text-xs text-warning"
                >
                  {{ effectiveQuota.message }}
                </p>
              </div>
            </div>
          </section>

          <!-- Provider Bindings & Identity -->
          <section class="studio-panel overflow-hidden">
            <div class="flex items-center justify-between gap-2 border-b border-border/70 px-4 py-3">
              <h2 class="text-sm font-bold text-foreground">
                {{ t('llmGateway.accountPage.bindingsCardTitle') }}
              </h2>
              <Badge variant="outline" class="font-mono text-xs text-studio-violet">
                pool:{{ account.vendor }}
              </Badge>
            </div>

            <div class="p-4">
              <dl class="divide-y divide-border/60 text-xs">
                <div class="flex items-center justify-between py-2">
                  <dt class="text-muted-foreground">Account Key</dt>
                  <dd class="font-mono text-foreground">{{ account.account_key }}</dd>
                </div>
                <div class="flex items-center justify-between py-2">
                  <dt class="text-muted-foreground">Vendor</dt>
                  <dd class="text-foreground">{{ vendorInfo?.display_name || account.vendor }}</dd>
                </div>
                <div class="flex items-center justify-between py-2">
                  <dt class="text-muted-foreground">Email</dt>
                  <dd class="text-foreground">{{ account.account_email || '—' }}</dd>
                </div>
                <div class="flex items-center justify-between py-2">
                  <dt class="text-muted-foreground">Binding</dt>
                  <dd class="font-mono text-foreground">pool:{{ account.vendor }}</dd>
                </div>
                <div class="flex items-center justify-between py-2">
                  <dt class="text-muted-foreground">{{ t('llmGateway.expiresAt') }}</dt>
                  <dd class="text-foreground">{{ formatTimestamp(account.expires_at) }}</dd>
                </div>
                <div v-if="maskedCredentialSummary(account)" class="flex items-center justify-between py-2">
                  <dt class="text-muted-foreground">Credential</dt>
                  <dd class="font-mono text-foreground">{{ maskedCredentialSummary(account) }}</dd>
                </div>
                <div v-if="account.bound_providers.length" class="py-2">
                  <dt class="mb-1.5 text-muted-foreground">
                    {{ t('llmGateway.col.boundProviders') }}
                  </dt>
                  <dd class="flex flex-wrap gap-1.5">
                    <RouterLink
                      v-for="bp in account.bound_providers"
                      :key="`${bp.provider_key}:${bp.binding_mode}`"
                      :to="`/providers/${encodeURIComponent(bp.provider_key)}`"
                      class="inline-flex items-center gap-1 rounded-md border border-border/80 bg-surface px-2 py-0.5 text-xs hover:border-studio-violet hover:text-studio-violet"
                    >
                      <span class="font-medium">{{ bp.display_name }}</span>
                      <span class="font-mono text-[10px] text-muted-foreground">
                        ({{ bp.binding_mode }})
                      </span>
                    </RouterLink>
                  </dd>
                </div>
              </dl>

              <div class="mt-3 pt-1">
                <Button
                  variant="outline"
                  size="sm"
                  class="h-7 px-2.5 text-xs"
                  :data-pool="account.vendor"
                  @click="
                    router.push({
                      name: 'llm-gateway-pool',
                      params: { vendor: account.vendor },
                    })
                  "
                >
                  {{ t('llmGateway.accountPage.openPoolBtn') }}
                  <ArrowRight class="size-3.5" aria-hidden="true" />
                </Button>
              </div>
            </div>
          </section>
        </div>

        <aside class="space-y-3.5">
          <section class="studio-panel overflow-hidden">
            <div class="border-b border-border/70 px-4 py-3">
              <h2 class="text-sm font-bold text-foreground">
                {{ t('llmGateway.accountPage.actionsCardTitle') }}
              </h2>
            </div>

            <div class="space-y-2.5 p-4">
              <div class="flex items-center justify-between gap-2 rounded-xl border border-border/70 bg-surface/40 p-3">
                <div>
                  <div class="text-xs font-bold text-foreground">
                    {{ t('llmGateway.accountPage.allowUseTitle') }}
                  </div>
                  <div class="text-[11px] text-muted-foreground">
                    {{ t('llmGateway.accountPage.allowUseDesc') }}
                  </div>
                </div>
                <button
                  type="button"
                  role="switch"
                  :aria-checked="account.enabled"
                  data-action="toggle-account"
                  :disabled="busy"
                  class="relative h-6 w-11 shrink-0 rounded-full border transition-colors"
                  :class="
                    account.enabled
                      ? 'border-studio-violet bg-studio-violet'
                      : 'border-border bg-muted'
                  "
                  @click="toggleEnabled"
                >
                  <span
                    class="block size-4 rounded-full bg-white transition-transform"
                    :class="account.enabled ? 'translate-x-5.5' : 'translate-x-0.5'"
                  />
                </button>
              </div>

              <Button
                variant="outline"
                size="sm"
                class="w-full justify-start"
                :disabled="busy"
                @click="openEditModal"
              >
                <Pencil class="size-3.5" aria-hidden="true" />
                {{ t('llmGateway.accountPage.editLabelPriorityBtn') }}
              </Button>

              <Button
                variant="outline"
                size="sm"
                class="w-full justify-start"
                :disabled="busy"
                @click="refreshAccount"
              >
                <RefreshCw class="size-3.5" aria-hidden="true" />
                {{ t('llmGateway.accountPage.refreshIdentityBtn') }}
              </Button>

              <Button
                v-if="account.status === 'cooling_down'"
                variant="outline"
                size="sm"
                class="w-full justify-start"
                :disabled="busy"
                @click="clearCooldown"
              >
                <Clock class="size-3.5" aria-hidden="true" />
                {{ t('llmGateway.clearCooldown') }}
              </Button>

              <Button
                variant="outline"
                size="sm"
                class="w-full justify-start border-danger/30 text-danger hover:bg-danger/10"
                data-action="ask-delete-account"
                :disabled="busy"
                @click="deleteOpen = true"
              >
                <Trash2 class="size-3.5" aria-hidden="true" />
                {{ t('llmGateway.deleteModal.confirm') }}
              </Button>
            </div>
          </section>

          <div class="flex items-start gap-2.5 rounded-xl border border-studio-violet/25 bg-studio-violet/5 p-3.5 text-xs">
            <Shield class="mt-0.5 size-4 shrink-0 text-studio-violet" aria-hidden="true" />
            <div>
              <div class="font-bold text-foreground">
                {{ t('llmGateway.accountPage.secretNoticeTitle') }}
              </div>
              <p class="mt-0.5 text-muted-foreground">
                {{ t('llmGateway.accountPage.secretNoticeDesc') }}
              </p>
            </div>
          </div>
        </aside>
      </div>
    </template>

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
          <Button type="button" variant="outline" @click="editOpen = false">
            {{ t('common.cancel') }}
          </Button>
          <Button type="submit" :disabled="busy">{{ t('common.saveChanges') }}</Button>
        </div>
      </form>
    </BaseModal>

    <ConfirmDialog
      v-model="deleteOpen"
      :title="t('llmGateway.deleteModal.title')"
      :description="account ? `${account.label} (${account.account_key})` : ''"
      :confirm-label="t('llmGateway.deleteModal.confirm')"
      destructive
      @confirm="confirmDeleteAccount"
    />
  </section>
</template>

<script setup lang="ts">
import { ArrowRight, Check, Copy, ExternalLink, KeyRound, Loader2, RefreshCw } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { llmGatewayApi } from '@/api/llm-gateway'
import type {
  GatewayAccountView,
  GatewayVendor,
  GatewayVendorInfo,
  OAuthLoginChallenge,
} from '@/api/types/llm-gateway'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import DetailHero from '@/components/admin/DetailHero.vue'
import { KeyRound as HeroIcon } from '@lucide/vue'
import { useI18n } from '@/composables/useI18n'
import { useErrorToast } from '@/composables/useNotify'
import { loginModeLabel, vendorMeta } from './llmGatewayUi'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()

const vendors = ref<GatewayVendorInfo[]>([])
const selectedVendor = ref<GatewayVendor>('openai_codex')
const label = ref('')
const challenge = ref<OAuthLoginChallenge | null>(null)
const callbackInput = ref('')
const cookieInput = ref('')
const completedAccount = ref<GatewayAccountView | null>(null)
const starting = ref(false)
const completing = ref(false)
const copied = ref(false)
const errorMessage = ref('')
useErrorToast(() => errorMessage.value)

let pollTimer: ReturnType<typeof setTimeout> | undefined
let activePollSessionId: string | null = null
let authPopup: Window | null = null

const currentStep = computed<1 | 2 | 3>(() => {
  if (completedAccount.value) return 3
  if (challenge.value) return 2
  return 1
})

const activeVendorInfo = computed(
  () => vendors.value.find((item) => item.vendor === selectedVendor.value) ?? null,
)

const isCookieVendor = computed(() => activeVendorInfo.value?.login_mode === 'cookie_paste')

function closeAuthPopup() {
  if (authPopup && !authPopup.closed) {
    try {
      authPopup.close()
    } catch {
      /* Ignore cross-origin window close errors. */
    }
  }
  authPopup = null
}

function stopPolling() {
  activePollSessionId = null
  if (pollTimer !== undefined) {
    clearTimeout(pollTimer)
    pollTimer = undefined
  }
}

function scheduleNextPoll(sessionId: string, intervalSecs: number) {
  if (
    activePollSessionId !== sessionId ||
    !challenge.value ||
    completedAccount.value
  ) {
    stopPolling()
    return
  }
  const isDeviceFlow = challenge.value.mode === 'device_code'
  const minSecs = isDeviceFlow ? 5 : 2
  const baseSecs = Math.max(minSecs, intervalSecs || minSecs)
  const delayMs = baseSecs * 1000 + (isDeviceFlow ? 300 : 0)

  pollTimer = setTimeout(async () => {
    if (
      activePollSessionId !== sessionId ||
      !challenge.value ||
      completedAccount.value
    ) {
      stopPolling()
      return
    }
    try {
      const status = await llmGatewayApi.pollOAuthSession(sessionId)
      if (activePollSessionId !== sessionId) return
      if (status.status === 'completed') {
        stopPolling()
        closeAuthPopup()
        completedAccount.value = status.account
        return
      }
      if (status.status === 'failed') {
        stopPolling()
        closeAuthPopup()
        errorMessage.value = status.error || 'OAuth session failed.'
        return
      }
      scheduleNextPoll(sessionId, status.poll_interval_secs || baseSecs)
    } catch (cause) {
      if (activePollSessionId !== sessionId) return
      const message = formatApiError(cause)
      const lower = message.toLowerCase()
      if (
        lower.includes('expired') ||
        lower.includes('not found') ||
        lower.includes('session_not_found') ||
        lower.includes('session_expired')
      ) {
        errorMessage.value = message
        stopPolling()
        return
      }
      scheduleNextPoll(sessionId, baseSecs)
    }
  }, delayMs)
}

function startPolling(sessionId: string, intervalSecs: number) {
  stopPolling()
  activePollSessionId = sessionId
  scheduleNextPoll(sessionId, intervalSecs)
}

function buildRedirectUri(vendor: GatewayVendor): string {
  if (vendor === 'openai_codex') {
    return 'http://localhost:1455/auth/callback'
  }
  const appPort =
    window.location.port || (window.location.protocol === 'https:' ? '443' : '80')
  return `http://localhost:${appPort}/api/admin/llm-gateway/oauth/callback`
}

async function handleImportCookies(trimmedLabel: string) {
  const cookies = cookieInput.value.trim()
  if (!cookies) {
    errorMessage.value = t('llmGateway.oauthModal.cookieRequired')
    return
  }
  starting.value = true
  errorMessage.value = ''
  try {
    const res = await llmGatewayApi.importCookieAccount({
      vendor: selectedVendor.value,
      label: trimmedLabel,
      cookies,
    })
    cookieInput.value = ''
    completedAccount.value = res.account
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    starting.value = false
  }
}

async function handleStartOAuth() {
  const trimmedLabel = label.value.trim()
  if (!trimmedLabel) {
    errorMessage.value = t('llmGateway.editModal.labelRequired')
    return
  }
  if (isCookieVendor.value) {
    await handleImportCookies(trimmedLabel)
    return
  }
  starting.value = true
  errorMessage.value = ''
  try {
    const redirectUri = buildRedirectUri(selectedVendor.value)
    const result = await llmGatewayApi.startOAuth({
      vendor: selectedVendor.value,
      label: trimmedLabel,
      redirect_uri: redirectUri,
    })
    challenge.value = result
    startPolling(result.session_id, result.poll_interval_secs)
    if (result.mode === 'pkce' && result.authorize_url) {
      authPopup = window.open(result.authorize_url, 'oauth_popup', 'width=640,height=720')
    }
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    starting.value = false
  }
}

function openAuthWindow() {
  if (challenge.value?.mode === 'device_code' && challenge.value.user_code && !copied.value) {
    void copyText(challenge.value.user_code)
  }
  const url =
    challenge.value?.mode === 'device_code'
      ? challenge.value.verification_uri
      : challenge.value?.authorize_url
  if (url) {
    authPopup = window.open(url, 'oauth_popup', 'width=640,height=720')
  }
}

async function copyText(value: string | null | undefined) {
  if (!value || !navigator.clipboard?.writeText) return
  try {
    await navigator.clipboard.writeText(value)
    copied.value = true
    setTimeout(() => {
      copied.value = false
    }, 1800)
  } catch {
    /* Ignore clipboard errors. */
  }
}

async function handleManualComplete() {
  if (!challenge.value) return
  const raw = callbackInput.value.trim()
  if (!raw) return
  completing.value = true
  errorMessage.value = ''
  try {
    const res = await llmGatewayApi.completeOAuthSession(challenge.value.session_id, {
      callback_url_or_code: raw,
    })
    stopPolling()
    closeAuthPopup()
    completedAccount.value = res.account
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    completing.value = false
  }
}

function onWindowMessage(event: MessageEvent) {
  if (!challenge.value || completedAccount.value) return
  if (event.origin !== window.location.origin) return
  const data = event.data as Record<string, unknown> | null
  if (!data || data.type !== 'llm_gateway_oauth_complete') return
  if (data.status === 'completed' && data.account && typeof data.account === 'object') {
    stopPolling()
    closeAuthPopup()
    completedAccount.value = data.account as GatewayAccountView
  } else if (data.status === 'failed') {
    errorMessage.value = String(data.message || data.error || 'OAuth callback error.')
  }
}

async function loadVendors() {
  try {
    const res = await llmGatewayApi.listVendors()
    vendors.value = res.vendors
    const qVendor = String(route.query.vendor || '') as GatewayVendor
    if (qVendor && res.vendors.some((v) => v.vendor === qVendor)) {
      selectedVendor.value = qVendor
    } else if (res.vendors[0]) {
      selectedVendor.value = res.vendors[0].vendor
    }
    const qLabel = String(route.query.label || '').trim()
    if (qLabel) {
      label.value = qLabel
    }
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  }
}

onMounted(() => {
  window.addEventListener('message', onWindowMessage)
  void loadVendors()
})

onBeforeUnmount(() => {
  stopPolling()
  closeAuthPopup()
  window.removeEventListener('message', onWindowMessage)
})
</script>

<template>
  <section class="space-y-4" data-testid="llm-gateway-connect-page">
    <DetailHero tone="gateway" :eyebrow="t('llmGateway.connectPage.eyebrow')" :title="t('llmGateway.connectPage.title')" :description="t('llmGateway.connectPage.subtitle')" :back-label="t('llmGateway.backToAccounts')" @back="router.push({ name: 'llm-gateway-accounts' })">
      <template #icon><HeroIcon :size="28" /></template>
    </DetailHero>

    <!-- 3 Wizard Steps -->
    <div class="grid gap-2.5 sm:grid-cols-3">
      <div
        class="rounded-xl border p-3 text-xs font-bold"
        :class="
          currentStep === 1
            ? 'border-studio-violet bg-studio-violet/10 text-studio-violet'
            : 'border-border/70 bg-surface/40 text-muted-foreground'
        "
      >
        {{ t('llmGateway.connectPage.step1') }}
      </div>
      <div
        class="rounded-xl border p-3 text-xs font-bold"
        :class="
          currentStep === 2
            ? 'border-studio-violet bg-studio-violet/10 text-studio-violet'
            : 'border-border/70 bg-surface/40 text-muted-foreground'
        "
      >
        {{ t('llmGateway.connectPage.step2') }}
      </div>
      <div
        class="rounded-xl border p-3 text-xs font-bold"
        :class="
          currentStep === 3
            ? 'border-studio-violet bg-studio-violet/10 text-studio-violet'
            : 'border-border/70 bg-surface/40 text-muted-foreground'
        "
      >
        {{ t('llmGateway.connectPage.step3') }}
      </div>
    </div>

    <div class="grid gap-4 lg:grid-cols-[minmax(0,1fr)_315px]">
      <section class="studio-panel overflow-hidden">
        <div class="flex items-center justify-between gap-2 border-b border-border/70 px-4 py-3">
          <div>
            <h2 class="text-sm font-bold text-foreground">
              {{
                currentStep === 1
                  ? 'Thiết lập kết nối'
                  : currentStep === 2
                    ? 'Xác thực tài khoản'
                    : 'Kết nối thành công'
              }}
            </h2>
            <p class="text-xs text-muted-foreground">
              {{ t('llmGateway.oauthModal.description') }}
            </p>
          </div>
          <Badge variant="outline" class="text-xs text-studio-violet">
            {{ currentStep }} / 3
          </Badge>
        </div>

        <div class="p-4">
          <!-- STEP 1 -->
          <form v-if="currentStep === 1" class="space-y-4" @submit.prevent="handleStartOAuth">
            <div class="space-y-2">
              <h3 class="text-xs font-bold text-foreground">
                {{ t('llmGateway.oauthModal.chooseVendor') }}
              </h3>
              <div class="grid gap-2.5 sm:grid-cols-2 lg:grid-cols-3">
                <button
                  v-for="v in vendors"
                  :key="v.vendor"
                  type="button"
                  :data-connect-vendor="v.vendor"
                  class="flex items-center gap-2.5 rounded-xl border p-3 text-left transition-colors"
                  :class="
                    selectedVendor === v.vendor
                      ? 'border-studio-violet bg-studio-violet/10'
                      : 'border-border/80 bg-surface/40 hover:bg-accent/30'
                  "
                  @click="selectedVendor = v.vendor"
                >
                  <span
                    class="inline-flex size-8 shrink-0 items-center justify-center rounded-lg border font-mono text-[11px] font-extrabold"
                    :class="vendorMeta(v.vendor).logoClass"
                  >
                    {{ vendorMeta(v.vendor).short }}
                  </span>
                  <div class="min-w-0">
                    <div class="truncate text-xs font-bold text-foreground">
                      {{ v.display_name }}
                    </div>
                    <div class="font-mono text-[10px] text-muted-foreground">
                      {{ v.pool_binding }}
                    </div>
                  </div>
                </button>
              </div>
            </div>

            <label class="block space-y-1.5">
              <span class="text-xs font-semibold">
                {{ t('llmGateway.oauthModal.label') }} *
              </span>
              <input
                v-model="label"
                data-input="connect-label"
                class="admin-input text-xs"
                :placeholder="t('llmGateway.oauthModal.labelPlaceholder')"
                maxlength="128"
                required
              />
            </label>

            <label v-if="isCookieVendor" class="block space-y-1.5">
              <span class="text-xs font-semibold">
                {{ t('llmGateway.oauthModal.cookieLabel') }} *
              </span>
              <textarea
                v-model="cookieInput"
                data-input="connect-cookies"
                rows="5"
                class="admin-input min-h-28 font-mono text-xs"
                autocomplete="off"
                spellcheck="false"
                placeholder="hatch_sess=...; hatch_gw=...; hatch_vml=...; hatch_native_auth_device=..."
              />
              <small class="block text-xs text-muted-foreground">
                {{ t('llmGateway.oauthModal.cookieHint') }}
              </small>
            </label>

            <p v-if="activeVendorInfo" class="text-xs text-muted-foreground">
              OAuth Mode:
              <strong class="text-foreground">{{ loginModeLabel(activeVendorInfo.login_mode) }}</strong>
              · Identity sẽ được thêm vào
              <code class="font-mono text-foreground">{{ activeVendorInfo.pool_binding }}</code>.
            </p>

            <div class="pt-1">
              <Button
                type="submit"
                size="sm"
                data-action="start-oauth"
                :disabled="starting"
              >
                <KeyRound class="size-4" aria-hidden="true" />
                {{
                  starting
                    ? t('llmGateway.oauthModal.starting')
                    : isCookieVendor
                      ? t('llmGateway.oauthModal.cookieSubmit')
                      : t('llmGateway.oauthModal.startBtn')
                }}
                <ArrowRight class="size-4" aria-hidden="true" />
              </Button>
            </div>
          </form>

          <!-- STEP 2 -->
          <div v-else-if="currentStep === 2 && challenge" class="space-y-4">
            <div class="flex items-center justify-between gap-2 rounded-xl border border-border/70 bg-surface/50 px-3.5 py-2.5 text-xs">
              <div>
                <span class="text-muted-foreground">{{ t('llmGateway.col.vendor') }}:</span>
                <strong class="ml-1.5">{{ activeVendorInfo?.display_name || challenge.vendor }}</strong>
                <span class="mx-2 text-muted-foreground">·</span>
                <span class="text-muted-foreground">Label:</span>
                <strong class="ml-1.5">{{ label }}</strong>
              </div>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                data-action="back-oauth"
                @click="challenge = null; stopPolling()"
              >
                {{ t('llmGateway.oauthModal.changeVendor') }}
              </Button>
            </div>

            <template v-if="challenge.mode === 'device_code'">
              <div class="space-y-3 rounded-xl border border-studio-violet/30 bg-studio-violet/5 p-4 text-center">
                <p class="text-xs uppercase tracking-wider text-muted-foreground">
                  {{ t('llmGateway.oauthModal.userCodeTitle') }}
                </p>
                <div class="flex items-center justify-center gap-2">
                  <code class="rounded-lg border bg-background px-4 py-2 font-mono text-2xl font-bold tracking-widest">
                    {{ challenge.user_code || '—' }}
                  </code>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    @click="copyText(challenge.user_code)"
                  >
                    <Copy class="size-4" aria-hidden="true" />
                    {{ copied ? t('llmGateway.oauthModal.copied') : t('llmGateway.oauthModal.copy') }}
                  </Button>
                </div>
                <p class="text-xs text-muted-foreground">
                  {{ t('llmGateway.oauthModal.deviceHint') }}
                </p>
                <div class="flex justify-center pt-1">
                  <Button type="button" size="sm" @click="openAuthWindow">
                    <ExternalLink class="size-4" aria-hidden="true" />
                    {{ t('llmGateway.oauthModal.openGithub') }}
                  </Button>
                </div>
              </div>

              <div class="flex items-center justify-center gap-2 py-2 text-xs text-muted-foreground">
                <Loader2 class="size-4 animate-spin text-studio-violet" aria-hidden="true" />
                <span>{{ t('llmGateway.oauthModal.waitingProvider') }}</span>
              </div>
            </template>

            <template v-else>
              <div class="space-y-3 rounded-xl border border-border/80 bg-surface/50 p-3.5">
                <div class="flex flex-wrap items-center justify-between gap-2">
                  <span class="text-xs font-semibold">
                    {{ t('llmGateway.oauthModal.step1Browser') }}
                  </span>
                  <div class="flex items-center gap-2">
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      @click="copyText(challenge.authorize_url)"
                    >
                      <Copy class="size-3.5" aria-hidden="true" />
                      {{ copied ? t('llmGateway.oauthModal.copied') : t('llmGateway.oauthModal.copyUrl') }}
                    </Button>
                    <Button type="button" size="sm" @click="openAuthWindow">
                      <ExternalLink class="size-3.5" aria-hidden="true" />
                      {{ t('llmGateway.oauthModal.openWindow') }}
                    </Button>
                  </div>
                </div>
                <p class="text-xs text-muted-foreground">
                  {{ t('llmGateway.oauthModal.pkceHint') }}
                </p>
              </div>

              <form class="space-y-3" @submit.prevent="handleManualComplete">
                <label class="block space-y-1.5">
                  <span class="text-xs font-semibold">
                    {{ t('llmGateway.oauthModal.step2Manual') }}
                  </span>
                  <input
                    v-model="callbackInput"
                    data-input="connect-code"
                    class="admin-input font-mono text-xs"
                    placeholder="http://localhost:.../callback?code=...&state=..."
                  />
                </label>

                <div class="flex items-center justify-between gap-2">
                  <span class="flex items-center gap-1.5 text-xs text-muted-foreground">
                    <RefreshCw class="size-3.5 animate-spin text-studio-violet" aria-hidden="true" />
                    {{ t('llmGateway.oauthModal.listening') }}
                  </span>
                  <Button
                    type="submit"
                    size="sm"
                    data-action="finish-oauth"
                    :disabled="completing || !callbackInput.trim()"
                  >
                    <Check class="size-4" aria-hidden="true" />
                    {{
                      completing
                        ? t('llmGateway.oauthModal.completing')
                        : t('llmGateway.oauthModal.completeBtn')
                    }}
                  </Button>
                </div>
              </form>
            </template>
          </div>

          <!-- STEP 3 -->
          <div v-else-if="completedAccount" class="space-y-4 py-6 text-center">
            <span class="mx-auto flex size-14 items-center justify-center rounded-full bg-success/15 text-success">
              <Check class="size-7" aria-hidden="true" />
            </span>
            <div>
              <h2 class="text-base font-bold text-foreground">
                {{ t('llmGateway.oauthModal.completedTitle') }}
              </h2>
              <p class="mt-1 text-xs text-muted-foreground">
                <strong>{{ completedAccount.label }}</strong>
                <span v-if="completedAccount.account_email">
                  ({{ completedAccount.account_email }})
                </span>
                — <code class="font-mono">pool:{{ completedAccount.vendor }}</code>
              </p>
            </div>
            <div>
              <Button
                size="sm"
                @click="
                  router.push({
                    name: 'llm-gateway-account',
                    params: { accountKey: completedAccount.account_key },
                  })
                "
              >
                Đến OAuth Account
                <ArrowRight class="size-4" aria-hidden="true" />
              </Button>
            </div>
          </div>
        </div>
      </section>

      <!-- Connection Guide Aside -->
      <aside class="studio-panel overflow-hidden">
        <div class="border-b border-border/70 px-4 py-3">
          <h2 class="text-sm font-bold text-foreground">
            {{ t('llmGateway.connectPage.guideTitle') }}
          </h2>
        </div>

        <div class="space-y-3.5 p-4">
          <div class="flex items-center gap-2.5">
            <span
              class="inline-flex size-9 shrink-0 items-center justify-center rounded-xl border font-mono text-xs font-extrabold"
              :class="vendorMeta(selectedVendor).logoClass"
            >
              {{ vendorMeta(selectedVendor).short }}
            </span>
            <div>
              <div class="text-xs font-bold text-foreground">
                {{ activeVendorInfo?.display_name || selectedVendor }}
              </div>
              <div class="text-[11px] text-muted-foreground">
                {{ loginModeLabel(activeVendorInfo?.login_mode) }}
              </div>
            </div>
          </div>

          <div class="h-px bg-border/60" />

          <div class="space-y-2.5 text-xs">
            <div class="rounded-xl border border-border/70 bg-surface/40 p-3">
              <div class="font-bold text-foreground">Step 1 · Vendor</div>
              <p class="mt-0.5 text-muted-foreground">
                Chọn Vendor Pool và đặt nhãn dễ nhận biết.
              </p>
            </div>
            <div class="rounded-xl border border-border/70 bg-surface/40 p-3">
              <div class="font-bold text-foreground">Step 2 · Authorize</div>
              <p class="mt-0.5 text-muted-foreground">
                {{
                  isCookieVendor
                    ? 'Dán cookie từ trình duyệt đã đăng nhập; Gateway kiểm tra phiên rồi lưu ngay.'
                    : activeVendorInfo?.login_mode === 'device_code'
                      ? 'Sử dụng GitHub Device Code flow.'
                      : 'Mở link và nhập callback code qua PKCE flow.'
                }}
              </p>
            </div>
            <div class="rounded-xl border border-border/70 bg-surface/40 p-3">
              <div class="font-bold text-foreground">Step 3 · Saved</div>
              <p class="mt-0.5 text-muted-foreground">
                Gateway lưu identity và hiển thị trạng thái / Quota.
              </p>
            </div>
          </div>
        </div>
      </aside>
    </div>
  </section>
</template>

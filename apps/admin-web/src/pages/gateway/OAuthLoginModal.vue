<script setup lang="ts">
import { Check, Copy, ExternalLink, KeyRound, Loader2, RefreshCw } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import { formatApiError } from '@/api/errors'
import { llmGatewayApi } from '@/api/llm-gateway'
import type {
  GatewayAccountView,
  GatewayVendor,
  GatewayVendorInfo,
  OAuthLoginChallenge,
} from '@/api/types/llm-gateway'
import BaseModal from '@/components/admin/BaseModal.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const props = withDefaults(
  defineProps<{
    vendors: GatewayVendorInfo[]
    initialVendor?: GatewayVendor
  }>(),
  {
    initialVendor: undefined,
  },
)

const open = defineModel<boolean>({ required: true })

const emit = defineEmits<{
  completed: [account: GatewayAccountView]
}>()

const { t } = useI18n()

const selectedVendor = ref<GatewayVendor>('openai_codex')
const label = ref('')
const challenge = ref<OAuthLoginChallenge | null>(null)
const callbackInput = ref('')
const completedAccount = ref<GatewayAccountView | null>(null)
const starting = ref(false)
const completing = ref(false)
const polling = ref(false)
const copied = ref(false)
const errorMessage = ref('')

let pollTimer: ReturnType<typeof setInterval> | undefined

const activeVendorInfo = computed(() =>
  props.vendors.find((item) => item.vendor === selectedVendor.value),
)

function stopPolling() {
  if (pollTimer) {
    clearInterval(pollTimer)
    pollTimer = undefined
  }
  polling.value = false
}

function resetState() {
  stopPolling()
  selectedVendor.value = props.initialVendor ?? props.vendors[0]?.vendor ?? 'openai_codex'
  label.value = ''
  challenge.value = null
  callbackInput.value = ''
  completedAccount.value = null
  starting.value = false
  completing.value = false
  copied.value = false
  errorMessage.value = ''
}

watch(open, (isOpen) => {
  if (isOpen) {
    resetState()
  } else {
    stopPolling()
  }
})

watch(
  () => props.initialVendor,
  (next) => {
    if (next && !challenge.value) {
      selectedVendor.value = next
    }
  },
)

function startPolling(sessionId: string, intervalSecs: number) {
  stopPolling()
  polling.value = true
  const ms = Math.max(2, intervalSecs || 2) * 1000
  pollTimer = setInterval(async () => {
    if (!open.value || !challenge.value || completedAccount.value) {
      stopPolling()
      return
    }
    try {
      const status = await llmGatewayApi.pollOAuthSession(sessionId)
      if (status.status === 'completed') {
        stopPolling()
        completedAccount.value = status.account
        emit('completed', status.account)
      } else if (status.status === 'failed') {
        stopPolling()
        errorMessage.value = status.error || 'OAuth session failed.'
      }
    } catch {
      /* Ignore transient poll errors while waiting for user completion. */
    }
  }, ms)
}

async function handleStartOAuth() {
  const trimmedLabel = label.value.trim()
  if (!trimmedLabel) {
    errorMessage.value = t('llmGateway.editModal.labelRequired')
    return
  }
  starting.value = true
  errorMessage.value = ''
  try {
    const redirectUri = `${window.location.origin}/api/admin/llm-gateway/oauth/callback`
    const result = await llmGatewayApi.startOAuth({
      vendor: selectedVendor.value,
      label: trimmedLabel,
      redirect_uri: redirectUri,
    })
    challenge.value = result
    startPolling(result.session_id, result.poll_interval_secs)
    if (result.mode === 'pkce' && result.authorize_url) {
      window.open(result.authorize_url, '_blank', 'width=640,height=720,noopener')
    }
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    starting.value = false
  }
}

function openAuthWindow() {
  const url =
    challenge.value?.mode === 'device_code'
      ? challenge.value.verification_uri
      : challenge.value?.authorize_url
  if (url) {
    window.open(url, '_blank', 'width=640,height=720,noopener')
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
    completedAccount.value = res.account
    emit('completed', res.account)
  } catch (cause) {
    errorMessage.value = formatApiError(cause)
  } finally {
    completing.value = false
  }
}

function onWindowMessage(event: MessageEvent) {
  if (!open.value || !challenge.value || completedAccount.value) return
  if (event.origin !== window.location.origin) return
  const data = event.data as Record<string, unknown> | null
  if (!data || data.type !== 'llm_gateway_oauth_complete') return
  if (data.status === 'completed' && data.account && typeof data.account === 'object') {
    stopPolling()
    const acc = data.account as GatewayAccountView
    completedAccount.value = acc
    emit('completed', acc)
  } else if (data.status === 'failed') {
    errorMessage.value = String(data.message || data.error || 'OAuth callback error.')
  }
}

onMounted(() => {
  window.addEventListener('message', onWindowMessage)
})

onBeforeUnmount(() => {
  stopPolling()
  window.removeEventListener('message', onWindowMessage)
})
</script>

<template>
  <BaseModal
    v-model="open"
    :title="t('llmGateway.oauthModal.title')"
    :description="t('llmGateway.oauthModal.description')"
    width-class="max-w-xl"
  >
    <div class="space-y-4">
      <p
        v-if="errorMessage"
        role="alert"
        class="rounded-lg border border-danger/30 bg-danger/5 px-3.5 py-2.5 text-sm text-danger"
      >
        {{ errorMessage }}
      </p>

      <!-- STEP 3: Completed -->
      <div v-if="completedAccount" class="space-y-4">
        <div class="flex items-start gap-3 rounded-xl border border-success/30 bg-success/5 p-4">
          <span
            class="flex size-9 shrink-0 items-center justify-center rounded-full bg-success/15 text-success"
          >
            <Check class="size-5" aria-hidden="true" />
          </span>
          <div class="min-w-0 flex-1 space-y-1">
            <h3 class="font-semibold text-foreground">
              {{ t('llmGateway.oauthModal.completedTitle') }}
            </h3>
            <p class="text-sm text-muted-foreground">
              <strong>{{ completedAccount.label }}</strong>
              <span v-if="completedAccount.account_email">
                ({{ completedAccount.account_email }})
              </span>
              — <code class="font-mono text-xs">pool:{{ completedAccount.vendor }}</code>
            </p>
          </div>
        </div>

        <div class="flex justify-end gap-2 pt-2">
          <Button type="button" @click="open = false">{{ t('llmGateway.oauthModal.done') }}</Button>
        </div>
      </div>

      <!-- STEP 1: Choose Vendor & Label -->
      <form v-else-if="!challenge" class="space-y-4" @submit.prevent="handleStartOAuth">
        <div class="space-y-1.5">
          <label class="block text-sm font-medium">{{ t('llmGateway.oauthModal.chooseVendor') }}</label>
          <div class="grid gap-2.5 sm:grid-cols-2">
            <button
              v-for="v in vendors"
              :key="v.vendor"
              type="button"
              class="flex flex-col items-start rounded-xl border p-3 text-left transition-colors"
              :class="
                selectedVendor === v.vendor
                  ? 'border-studio-violet bg-studio-violet/10'
                  : 'border-border/80 bg-surface/50 hover:bg-accent/40'
              "
              @click="selectedVendor = v.vendor"
            >
              <div class="flex w-full items-center justify-between gap-2">
                <span class="font-semibold text-sm">{{ v.display_name }}</span>
                <Badge variant="outline" class="font-mono text-[10px]">
                  {{ v.login_mode === 'device_code' ? 'Device Code' : 'PKCE' }}
                </Badge>
              </div>
              <span class="mt-1 font-mono text-xs text-muted-foreground">{{ v.pool_binding }}</span>
            </button>
          </div>
        </div>

        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('llmGateway.oauthModal.label') }} <span class="text-danger">*</span></span>
          <input
            v-model="label"
            class="admin-input"
            :placeholder="t('llmGateway.oauthModal.labelPlaceholder')"
            maxlength="128"
            required
          />
          <small class="block text-xs text-muted-foreground">
            {{ t('llmGateway.oauthModal.labelHint') }}
          </small>
        </label>

        <div
          v-if="activeVendorInfo"
          class="rounded-lg border border-border/70 bg-surface/60 px-3.5 py-2.5 text-xs text-muted-foreground"
        >
          {{ t('llmGateway.oauthModal.defaultEndpoint') }}:
          <code class="font-mono text-foreground">{{ activeVendorInfo.default_base_url }}</code>
        </div>

        <div class="flex justify-end gap-2 pt-2">
          <Button type="button" variant="outline" @click="open = false">{{ t('common.cancel') }}</Button>
          <Button type="submit" :disabled="starting">
            <KeyRound class="size-4" aria-hidden="true" />
            {{ starting ? t('llmGateway.oauthModal.starting') : t('llmGateway.oauthModal.startBtn') }}
          </Button>
        </div>
      </form>

      <!-- STEP 2: Challenge Active (PKCE or Device Code) -->
      <div v-else class="space-y-4">
        <div class="flex items-center justify-between gap-2 rounded-lg border border-border/70 bg-surface/60 px-3.5 py-2.5">
          <div class="text-sm">
            <span class="text-muted-foreground">{{ t('llmGateway.col.vendor') }}:</span>
            <strong class="ml-1.5">{{ activeVendorInfo?.display_name || challenge.vendor }}</strong>
            <span class="mx-2 text-muted-foreground">·</span>
            <span class="text-muted-foreground">Label:</span>
            <strong class="ml-1.5">{{ label }}</strong>
          </div>
          <Button type="button" variant="ghost" size="sm" @click="challenge = null; stopPolling()">
            {{ t('llmGateway.oauthModal.changeVendor') }}
          </Button>
        </div>

        <!-- Device Code Flow -->
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
              <Button type="button" @click="openAuthWindow">
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

        <!-- Authorization Code + PKCE Flow -->
        <template v-else>
          <div class="space-y-3 rounded-xl border border-border/80 bg-surface/50 p-4">
            <div class="flex flex-wrap items-center justify-between gap-2">
              <span class="text-sm font-medium">{{ t('llmGateway.oauthModal.step1Browser') }}</span>
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
              <span class="text-sm font-medium">
                {{ t('llmGateway.oauthModal.step2Manual') }}
              </span>
              <input
                v-model="callbackInput"
                class="admin-input font-mono text-xs"
                placeholder="http://localhost:.../callback?code=...&state=..."
              />
            </label>
            <div class="flex items-center justify-between gap-2">
              <span class="flex items-center gap-1.5 text-xs text-muted-foreground">
                <RefreshCw class="size-3.5 animate-spin text-studio-violet" aria-hidden="true" />
                {{ t('llmGateway.oauthModal.listening') }}
              </span>
              <div class="flex items-center gap-2">
                <Button type="button" variant="outline" @click="open = false">{{ t('common.close') }}</Button>
                <Button type="submit" :disabled="completing || !callbackInput.trim()">
                  {{ completing ? t('llmGateway.oauthModal.completing') : t('llmGateway.oauthModal.completeBtn') }}
                </Button>
              </div>
            </div>
          </form>
        </template>
      </div>
    </div>
  </BaseModal>
</template>

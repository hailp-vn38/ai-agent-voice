<script setup lang="ts">
import { Bell, CheckCircle2, MessageSquare, Send, Sparkles, XCircle } from '@lucide/vue'
import { onMounted, ref } from 'vue'

import { isNotificationsDemoMode, notificationsApi } from '@/api/notifications'
import {
  ALL_NOTIFICATION_TOPICS,
  type ExternalChannelKind,
  type NotificationSettings,
  type NotificationSettingsUpdate,
  type NotificationTopic,
  type TestChannelOutput,
} from '@/api/types/notifications'
import { useI18n } from '@/composables/useI18n'
import type { MessageKey } from '@/i18n/messages'

const { t } = useI18n()

const loading = ref(true)
const saving = ref(false)
const demoMode = ref(false)
const revision = ref(1)
const savedBanner = ref('')
const errorBanner = ref('')

// Web UI state
const toastEnabled = ref(true)
const retentionDays = ref(30)

// Telegram state
const tgEnabled = ref(false)
const tgHasSavedToken = ref(false)
const tgTokenHint = ref<string | null>(null)
const tgTokenInput = ref('')
const tgClearToken = ref(false)
const tgChatId = ref('')
const tgThreadId = ref<string>('')
const tgTopics = ref<NotificationTopic[]>([])
const tgTesting = ref(false)
const tgTestResult = ref<TestChannelOutput | null>(null)

// Zalo state
const zaloEnabled = ref(false)
const zaloHasSavedToken = ref(false)
const zaloTokenHint = ref<string | null>(null)
const zaloTokenInput = ref('')
const zaloClearToken = ref(false)
const zaloRecipientId = ref('')
const zaloApiBaseUrl = ref('https://bot-api.zapps.me')
const zaloTopics = ref<NotificationTopic[]>([])
const zaloTesting = ref(false)
const zaloTestResult = ref<TestChannelOutput | null>(null)

function applyLoadedSettings(data: NotificationSettings): void {
  revision.value = data.revision
  demoMode.value = isNotificationsDemoMode()

  toastEnabled.value = data.web_ui.toast_enabled
  retentionDays.value = data.web_ui.retention_days

  tgEnabled.value = data.telegram.enabled
  tgHasSavedToken.value = data.telegram.has_saved_token
  tgTokenHint.value = data.telegram.credential_metadata?.masked_key ?? null
  tgTokenInput.value = ''
  tgClearToken.value = false
  tgChatId.value = data.telegram.chat_id
  tgThreadId.value = data.telegram.thread_id != null ? String(data.telegram.thread_id) : ''
  tgTopics.value = [...data.telegram.subscribed_topics]

  zaloEnabled.value = data.zalo.enabled
  zaloHasSavedToken.value = data.zalo.has_saved_token
  zaloTokenHint.value = data.zalo.credential_metadata?.masked_key ?? null
  zaloTokenInput.value = ''
  zaloClearToken.value = false
  zaloRecipientId.value = data.zalo.recipient_id
  zaloTopics.value = [...data.zalo.subscribed_topics]
}

async function load(): Promise<void> {
  loading.value = true
  errorBanner.value = ''
  try {
    const data = await notificationsApi.settings()
    applyLoadedSettings(data)
  } catch (err) {
    errorBanner.value = err instanceof Error ? err.message : String(err)
  } finally {
    loading.value = false
  }
}

function toggleTopic(list: NotificationTopic[], topic: NotificationTopic): NotificationTopic[] {
  return list.includes(topic) ? list.filter(item => item !== topic) : [...list, topic]
}

function resolveBotTokenPatch(inputVal: string, clearFlag: boolean): string | null | undefined {
  if (clearFlag) return null
  const trimmed = inputVal.trim()
  if (trimmed.length > 0) return trimmed
  return undefined
}

async function saveSettings(): Promise<void> {
  saving.value = true
  savedBanner.value = ''
  errorBanner.value = ''
  try {
    const parsedThread = tgThreadId.value.trim() ? Number(tgThreadId.value.trim()) : null
    const payload: NotificationSettingsUpdate = {
      web_ui: {
        toast_enabled: toastEnabled.value,
        retention_days: Number(retentionDays.value) || 30,
      },
      telegram: {
        enabled: tgEnabled.value,
        chat_id: tgChatId.value.trim(),
        thread_id: Number.isFinite(parsedThread) ? parsedThread : null,
        subscribed_topics: [...tgTopics.value],
        bot_token: resolveBotTokenPatch(tgTokenInput.value, tgClearToken.value),
      },
      zalo: {
        enabled: zaloEnabled.value,
        recipient_id: zaloRecipientId.value.trim(),
        subscribed_topics: [...zaloTopics.value],
        bot_token: resolveBotTokenPatch(zaloTokenInput.value, zaloClearToken.value),
      },
    }
    const updated = await notificationsApi.updateSettings(payload, revision.value)
    applyLoadedSettings(updated)
    savedBanner.value = t('notifications.settings.saved', { revision: updated.revision })
  } catch (err) {
    errorBanner.value = err instanceof Error ? err.message : String(err)
  } finally {
    saving.value = false
  }
}

async function runChannelTest(channel: ExternalChannelKind): Promise<void> {
  if (channel === 'telegram') {
    tgTesting.value = true
    tgTestResult.value = null
    try {
      const parsedThread = tgThreadId.value.trim() ? Number(tgThreadId.value.trim()) : null
      tgTestResult.value = await notificationsApi.testChannel('telegram', {
        chat_id: tgChatId.value.trim() || undefined,
        thread_id: Number.isFinite(parsedThread) ? parsedThread : null,
        bot_token: tgTokenInput.value.trim() || undefined,
      })
    } finally {
      tgTesting.value = false
    }
    return
  }

  zaloTesting.value = true
  zaloTestResult.value = null
  try {
    zaloTestResult.value = await notificationsApi.testChannel('zalo', {
      recipient_id: zaloRecipientId.value.trim() || undefined,
      bot_token: zaloTokenInput.value.trim() || undefined,
    })
  } finally {
    zaloTesting.value = false
  }
}

function topicLabel(topic: NotificationTopic): string {
  return t(`notifications.topic.${topic}` as MessageKey)
}

onMounted(() => {
  void load()
})
</script>

<template>
  <section
    class="rounded-xl border border-[var(--border)] bg-[var(--bg-elevated)] p-6 shadow-sm space-y-5"
  >
    <div class="flex flex-wrap items-start justify-between gap-4">
      <div>
        <p class="text-[11px] font-semibold uppercase tracking-[0.14em] text-[var(--text-muted)]">
          {{ t('notifications.settings.eyebrow') }}
        </p>
        <div class="mt-1 flex items-center gap-2">
          <h2 class="text-base font-semibold text-[var(--text)]">
            {{ t('notifications.settings.title') }}
          </h2>
          <span
            v-if="demoMode"
            class="inline-flex items-center gap-1 rounded-full border border-amber-500/30 bg-amber-500/10 px-2.5 py-0.5 text-[11px] font-medium text-amber-500"
          >
            <Sparkles class="h-3 w-3" />
            <span>{{ t('notifications.demoBadge') }}</span>
          </span>
        </div>
        <p class="mt-1 text-xs text-[var(--text-secondary)]">
          {{ t('notifications.settings.description') }}
        </p>
      </div>
    </div>

    <!-- Demo Mode Notice -->
    <div
      v-if="demoMode"
      data-testid="notification-settings-demo-banner"
      class="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3.5 py-2.5 text-xs text-amber-500"
    >
      {{ t('notifications.settings.demoHint') }}
    </div>

    <!-- Save / Error Banners -->
    <div
      v-if="savedBanner"
      data-testid="notification-settings-saved"
      class="rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3.5 py-2.5 text-xs font-medium text-emerald-500"
    >
      {{ savedBanner }}
    </div>
    <div
      v-if="errorBanner"
      class="rounded-lg border border-rose-500/30 bg-rose-500/10 px-3.5 py-2.5 text-xs font-medium text-rose-500"
    >
      {{ errorBanner }}
    </div>

    <div class="grid gap-4 lg:grid-cols-3">
      <!-- 1. Web UI Default Channel -->
      <div class="flex flex-col justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-subtle)] p-4 space-y-4">
        <div class="space-y-3">
          <div class="flex items-center justify-between gap-2">
            <div class="flex items-center gap-2">
              <Bell class="h-4 w-4 text-sky-500" />
              <h3 class="text-sm font-semibold text-[var(--text)]">
                {{ t('notifications.settings.webUiTitle') }}
              </h3>
            </div>
            <span class="rounded-full border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 text-[10px] font-semibold text-emerald-500">
              {{ t('notifications.settings.webUiBadge') }}
            </span>
          </div>

          <label class="flex items-center justify-between gap-3 rounded-lg border border-[var(--border)] bg-[var(--bg-elevated)] px-3 py-2 text-xs">
            <span class="text-[var(--text)]">{{ t('notifications.settings.toastEnabled') }}</span>
            <input
              v-model="toastEnabled"
              data-testid="webui-toast-toggle"
              type="checkbox"
              class="h-4 w-4 rounded border-[var(--border)]"
            />
          </label>

          <label class="block space-y-1 text-xs">
            <span class="font-medium text-[var(--text-secondary)]">
              {{ t('notifications.settings.retentionDays') }}
            </span>
            <input
              v-model.number="retentionDays"
              data-testid="webui-retention-days"
              type="text"
              inputmode="numeric"
              class="h-9 w-full rounded-lg border border-[var(--border)] bg-[var(--bg-elevated)] px-3 text-xs text-[var(--text)]"
            />
          </label>
        </div>
      </div>

      <!-- 2. Telegram Bot Channel -->
      <div class="flex flex-col justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-subtle)] p-4 space-y-4">
        <div class="space-y-3">
          <div class="flex items-center justify-between gap-2">
            <div class="flex items-center gap-2">
              <Send class="h-4 w-4 text-sky-500" />
              <h3 class="text-sm font-semibold text-[var(--text)]">
                {{ t('notifications.settings.telegramTitle') }}
              </h3>
            </div>
            <label class="inline-flex items-center gap-1.5 text-xs font-medium text-[var(--text-secondary)]">
              <input
                v-model="tgEnabled"
                data-testid="telegram-enabled-toggle"
                type="checkbox"
                class="h-4 w-4 rounded border-[var(--border)]"
              />
              <span>{{ t('notifications.settings.channelEnabled') }}</span>
            </label>
          </div>

          <!-- Bot Token with Patch<SecretValue> semantics -->
          <div class="space-y-1.5 text-xs">
            <div class="flex items-center justify-between gap-2">
              <span class="font-medium text-[var(--text-secondary)]">
                {{ t('notifications.settings.botToken') }}
              </span>
              <button
                v-if="tgHasSavedToken && !tgClearToken"
                type="button"
                data-testid="telegram-clear-token"
                class="text-[11px] font-medium text-rose-500 hover:underline"
                @click="tgClearToken = true; tgTokenInput = ''"
              >
                {{ t('notifications.settings.clearToken') }}
              </button>
              <button
                v-else-if="tgClearToken"
                type="button"
                data-testid="telegram-undo-clear-token"
                class="text-[11px] font-medium text-sky-500 hover:underline"
                @click="tgClearToken = false"
              >
                {{ t('notifications.settings.undoClearToken') }}
              </button>
            </div>

            <input
              v-model="tgTokenInput"
              data-testid="telegram-bot-token"
              type="password"
              autocomplete="off"
              :placeholder="
                tgHasSavedToken && !tgClearToken
                  ? t('notifications.settings.botTokenKeepPlaceholder', {
                      hint: tgTokenHint ?? '••••',
                    })
                  : t('notifications.settings.botTokenNewPlaceholder')
              "
              class="h-9 w-full rounded-lg border border-[var(--border)] bg-[var(--bg-elevated)] px-3 text-xs text-[var(--text)]"
              @input="tgTokenInput && (tgClearToken = false)"
            />
            <p
              v-if="tgClearToken"
              data-testid="telegram-clear-notice"
              class="text-[11px] text-amber-500"
            >
              {{ t('notifications.settings.tokenWillClear') }}
            </p>
          </div>

          <div class="grid grid-cols-2 gap-2">
            <label class="block space-y-1 text-xs">
              <span class="font-medium text-[var(--text-secondary)]">
                {{ t('notifications.settings.telegramTargetId') }}
              </span>
              <input
                v-model="tgChatId"
                data-testid="telegram-chat-id"
                type="text"
                placeholder="-100123456789"
                class="h-9 w-full rounded-lg border border-[var(--border)] bg-[var(--bg-elevated)] px-2.5 text-xs text-[var(--text)]"
              />
            </label>

            <label class="block space-y-1 text-xs">
              <span class="font-medium text-[var(--text-secondary)]">
                {{ t('notifications.settings.telegramThreadId') }}
              </span>
              <input
                v-model="tgThreadId"
                data-testid="telegram-thread-id"
                type="text"
                placeholder="42"
                class="h-9 w-full rounded-lg border border-[var(--border)] bg-[var(--bg-elevated)] px-2.5 text-xs text-[var(--text)]"
              />
            </label>
          </div>

          <!-- Subscribed Topics -->
          <div class="space-y-1.5">
            <p class="text-xs font-medium text-[var(--text-secondary)]">
              {{ t('notifications.settings.subscribedTopics') }}
            </p>
            <div class="grid grid-cols-2 gap-1.5">
              <label
                v-for="topic in ALL_NOTIFICATION_TOPICS"
                :key="`tg-${topic}`"
                class="flex items-center gap-1.5 rounded-md border border-[var(--border)] bg-[var(--bg-elevated)] px-2 py-1 text-[11px] text-[var(--text)]"
              >
                <input
                  :data-testid="`telegram-topic-${topic}`"
                  type="checkbox"
                  :checked="tgTopics.includes(topic)"
                  class="h-3.5 w-3.5 rounded border-[var(--border)]"
                  @change="tgTopics = toggleTopic(tgTopics, topic)"
                />
                <span class="truncate">{{ topicLabel(topic) }}</span>
              </label>
            </div>
          </div>
        </div>

        <!-- Test Telegram Channel -->
        <div class="space-y-2 border-t border-[var(--border)] pt-3">
          <button
            type="button"
            data-testid="telegram-test-btn"
            class="inline-flex h-8 w-full items-center justify-center gap-1.5 rounded-lg border border-[var(--border)] bg-[var(--bg-elevated)] px-3 text-xs font-medium text-[var(--text)] transition-colors hover:bg-[var(--bg-hover)] disabled:opacity-50"
            :disabled="tgTesting"
            @click="runChannelTest('telegram')"
          >
            <Send class="h-3.5 w-3.5" />
            <span>
              {{
                tgTesting
                  ? t('notifications.settings.testingChannel')
                  : t('notifications.settings.testChannel')
              }}
            </span>
          </button>

          <div
            v-if="tgTestResult"
            data-testid="telegram-test-result"
            class="flex items-start gap-1.5 rounded-lg border p-2 text-[11px]"
            :class="
              tgTestResult.ok
                ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-500'
                : 'border-rose-500/30 bg-rose-500/10 text-rose-500'
            "
          >
            <CheckCircle2 v-if="tgTestResult.ok" class="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <XCircle v-else class="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>
              {{
                tgTestResult.ok
                  ? `OK (${tgTestResult.external_message_id})`
                  : tgTestResult.error
              }}
            </span>
          </div>
        </div>
      </div>

      <!-- 3. Zalo Bot Channel -->
      <div class="flex flex-col justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-subtle)] p-4 space-y-4">
        <div class="space-y-3">
          <div class="flex items-center justify-between gap-2">
            <div class="flex items-center gap-2">
              <MessageSquare class="h-4 w-4 text-blue-500" />
              <h3 class="text-sm font-semibold text-[var(--text)]">
                {{ t('notifications.settings.zaloTitle') }}
              </h3>
            </div>
            <label class="inline-flex items-center gap-1.5 text-xs font-medium text-[var(--text-secondary)]">
              <input
                v-model="zaloEnabled"
                data-testid="zalo-enabled-toggle"
                type="checkbox"
                class="h-4 w-4 rounded border-[var(--border)]"
              />
              <span>{{ t('notifications.settings.channelEnabled') }}</span>
            </label>
          </div>

          <!-- Bot Token with Patch<SecretValue> semantics -->
          <div class="space-y-1.5 text-xs">
            <div class="flex items-center justify-between gap-2">
              <span class="font-medium text-[var(--text-secondary)]">
                {{ t('notifications.settings.botToken') }}
              </span>
              <button
                v-if="zaloHasSavedToken && !zaloClearToken"
                type="button"
                data-testid="zalo-clear-token"
                class="text-[11px] font-medium text-rose-500 hover:underline"
                @click="zaloClearToken = true; zaloTokenInput = ''"
              >
                {{ t('notifications.settings.clearToken') }}
              </button>
              <button
                v-else-if="zaloClearToken"
                type="button"
                data-testid="zalo-undo-clear-token"
                class="text-[11px] font-medium text-sky-500 hover:underline"
                @click="zaloClearToken = false"
              >
                {{ t('notifications.settings.undoClearToken') }}
              </button>
            </div>

            <input
              v-model="zaloTokenInput"
              data-testid="zalo-bot-token"
              type="password"
              autocomplete="off"
              :placeholder="
                zaloHasSavedToken && !zaloClearToken
                  ? t('notifications.settings.botTokenKeepPlaceholder', {
                      hint: zaloTokenHint ?? '••••',
                    })
                  : t('notifications.settings.botTokenNewPlaceholder')
              "
              class="h-9 w-full rounded-lg border border-[var(--border)] bg-[var(--bg-elevated)] px-3 text-xs text-[var(--text)]"
              @input="zaloTokenInput && (zaloClearToken = false)"
            />
            <p
              v-if="zaloClearToken"
              data-testid="zalo-clear-notice"
              class="text-[11px] text-amber-500"
            >
              {{ t('notifications.settings.tokenWillClear') }}
            </p>
          </div>

          <div class="grid grid-cols-2 gap-2">
            <label class="block space-y-1 text-xs">
              <span class="font-medium text-[var(--text-secondary)]">
                {{ t('notifications.settings.zaloTargetId') }}
              </span>
              <input
                v-model="zaloRecipientId"
                data-testid="zalo-recipient-id"
                type="text"
                placeholder="zalo-user-or-group"
                class="h-9 w-full rounded-lg border border-[var(--border)] bg-[var(--bg-elevated)] px-2.5 text-xs text-[var(--text)]"
              />
            </label>

            <label class="block space-y-1 text-xs">
              <span class="font-medium text-[var(--text-secondary)]">
                {{ t('notifications.settings.apiBaseUrl') }}
              </span>
              <input
                :value="zaloApiBaseUrl"
                type="text"
                readonly
                disabled
                class="h-9 w-full rounded-lg border border-[var(--border)] bg-[var(--bg-subtle)] px-2.5 text-xs text-[var(--text-muted)]"
              />
            </label>
          </div>

          <!-- Subscribed Topics -->
          <div class="space-y-1.5">
            <p class="text-xs font-medium text-[var(--text-secondary)]">
              {{ t('notifications.settings.subscribedTopics') }}
            </p>
            <div class="grid grid-cols-2 gap-1.5">
              <label
                v-for="topic in ALL_NOTIFICATION_TOPICS"
                :key="`zalo-${topic}`"
                class="flex items-center gap-1.5 rounded-md border border-[var(--border)] bg-[var(--bg-elevated)] px-2 py-1 text-[11px] text-[var(--text)]"
              >
                <input
                  :data-testid="`zalo-topic-${topic}`"
                  type="checkbox"
                  :checked="zaloTopics.includes(topic)"
                  class="h-3.5 w-3.5 rounded border-[var(--border)]"
                  @change="zaloTopics = toggleTopic(zaloTopics, topic)"
                />
                <span class="truncate">{{ topicLabel(topic) }}</span>
              </label>
            </div>
          </div>
        </div>

        <!-- Test Zalo Channel -->
        <div class="space-y-2 border-t border-[var(--border)] pt-3">
          <button
            type="button"
            data-testid="zalo-test-btn"
            class="inline-flex h-8 w-full items-center justify-center gap-1.5 rounded-lg border border-[var(--border)] bg-[var(--bg-elevated)] px-3 text-xs font-medium text-[var(--text)] transition-colors hover:bg-[var(--bg-hover)] disabled:opacity-50"
            :disabled="zaloTesting"
            @click="runChannelTest('zalo')"
          >
            <MessageSquare class="h-3.5 w-3.5" />
            <span>
              {{
                zaloTesting
                  ? t('notifications.settings.testingChannel')
                  : t('notifications.settings.testChannel')
              }}
            </span>
          </button>

          <div
            v-if="zaloTestResult"
            data-testid="zalo-test-result"
            class="flex items-start gap-1.5 rounded-lg border p-2 text-[11px]"
            :class="
              zaloTestResult.ok
                ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-500'
                : 'border-rose-500/30 bg-rose-500/10 text-rose-500'
            "
          >
            <CheckCircle2 v-if="zaloTestResult.ok" class="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <XCircle v-else class="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>
              {{
                zaloTestResult.ok
                  ? `OK (${zaloTestResult.external_message_id})`
                  : zaloTestResult.error
              }}
            </span>
          </div>
        </div>
      </div>
    </div>

    <!-- Save Footer -->
    <div class="flex items-center justify-end border-t border-[var(--border)] pt-4">
      <button
        type="button"
        data-testid="notification-settings-save"
        class="h-9 rounded-lg bg-[var(--text)] px-4 text-xs font-semibold text-[var(--bg)] disabled:opacity-50"
        :disabled="saving || loading"
        @click="saveSettings"
      >
        {{ saving ? t('notifications.settings.saving') : t('notifications.settings.save') }}
      </button>
    </div>
  </section>
</template>

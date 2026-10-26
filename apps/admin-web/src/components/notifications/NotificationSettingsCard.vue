<script setup lang="ts">
import { Bell, CheckCircle2, MessageSquare, Send, XCircle } from '@lucide/vue'
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
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
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
  <section class="studio-panel space-y-5 p-5" data-testid="notification-settings-card">
    <div class="flex flex-wrap items-start justify-between gap-4">
      <div class="flex items-start gap-3">
        <div class="grid size-10 shrink-0 place-items-center rounded-xl bg-studio-violet/10 text-studio-violet">
          <Bell class="size-5" />
        </div>
        <div>
          <div class="flex flex-wrap items-center gap-2">
            <h2 class="font-semibold text-foreground">
              {{ t('notifications.settings.title') }}
            </h2>
            <Badge v-if="demoMode" variant="secondary">
              {{ t('notifications.demoBadge') }}
            </Badge>
          </div>
          <p class="mt-1 text-sm text-muted-foreground">
            {{ t('notifications.settings.description') }}
          </p>
        </div>
      </div>

      <Button
        type="button"
        size="sm"
        data-testid="notification-settings-save"
        :disabled="saving || loading"
        @click="saveSettings"
      >
        {{ saving ? t('notifications.settings.saving') : t('notifications.settings.save') }}
      </Button>
    </div>

    <!-- Demo Mode Notice -->
    <div
      v-if="demoMode"
      data-testid="notification-settings-demo-banner"
      class="rounded-xl border border-warning/30 bg-warning/5 px-3.5 py-2.5 text-xs text-warning-foreground"
    >
      {{ t('notifications.settings.demoHint') }}
    </div>

    <!-- Save / Error Banners -->
    <div
      v-if="savedBanner"
      data-testid="notification-settings-saved"
      class="rounded-xl border border-success/30 bg-success/5 px-3.5 py-2.5 text-xs font-medium text-success-foreground"
    >
      {{ savedBanner }}
    </div>
    <div
      v-if="errorBanner"
      class="rounded-xl border border-danger/30 bg-danger/5 px-3.5 py-2.5 text-xs font-medium text-danger-foreground"
    >
      {{ errorBanner }}
    </div>

    <div class="grid gap-4 lg:grid-cols-3">
      <!-- 1. Web UI Default Channel -->
      <div class="flex flex-col justify-between rounded-xl border bg-background p-4 space-y-4">
        <div class="space-y-3">
          <div class="flex items-center justify-between gap-2">
            <div class="flex items-center gap-2">
              <Bell class="size-4 text-studio-violet" />
              <h3 class="text-sm font-semibold text-foreground">
                {{ t('notifications.settings.webUiTitle') }}
              </h3>
            </div>
            <Badge variant="outline">
              {{ t('notifications.settings.webUiBadge') }}
            </Badge>
          </div>

          <label class="flex cursor-pointer items-center justify-between gap-3 rounded-lg border bg-card px-3 py-2.5 text-xs">
            <span class="font-medium text-foreground">{{ t('notifications.settings.toastEnabled') }}</span>
            <input
              v-model="toastEnabled"
              data-testid="webui-toast-toggle"
              type="checkbox"
              class="size-4 accent-primary"
            />
          </label>

          <label class="block space-y-1 text-xs">
            <span class="font-medium text-muted-foreground">
              {{ t('notifications.settings.retentionDays') }}
            </span>
            <input
              v-model.number="retentionDays"
              data-testid="webui-retention-days"
              type="text"
              inputmode="numeric"
              class="admin-input h-9 text-xs"
            />
          </label>
        </div>
      </div>

      <!-- 2. Telegram Bot Channel -->
      <div class="flex flex-col justify-between rounded-xl border bg-background p-4 space-y-4">
        <div class="space-y-3">
          <div class="flex items-center justify-between gap-2">
            <div class="flex items-center gap-2">
              <Send class="size-4 text-studio-cyan" />
              <h3 class="text-sm font-semibold text-foreground">
                {{ t('notifications.settings.telegramTitle') }}
              </h3>
            </div>
            <label class="inline-flex cursor-pointer items-center gap-2 text-xs font-medium">
              <span class="text-muted-foreground">{{ t('notifications.settings.channelEnabled') }}</span>
              <input
                v-model="tgEnabled"
                data-testid="telegram-enabled-toggle"
                type="checkbox"
                class="size-4 accent-primary"
              />
            </label>
          </div>

          <!-- Bot Token with Patch<SecretValue> semantics -->
          <div class="space-y-1 text-xs">
            <div class="flex items-center justify-between gap-2">
              <span class="font-medium text-muted-foreground">
                {{ t('notifications.settings.botToken') }}
              </span>
              <button
                v-if="tgHasSavedToken && !tgClearToken"
                type="button"
                data-testid="telegram-clear-token"
                class="text-[11px] font-medium text-danger-foreground hover:underline"
                @click="tgClearToken = true; tgTokenInput = ''"
              >
                {{ t('notifications.settings.clearToken') }}
              </button>
              <button
                v-else-if="tgClearToken"
                type="button"
                data-testid="telegram-undo-clear-token"
                class="text-[11px] font-medium text-primary hover:underline"
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
              class="admin-input h-9 text-xs"
              @input="tgTokenInput && (tgClearToken = false)"
            />
            <p
              v-if="tgClearToken"
              data-testid="telegram-clear-notice"
              class="text-[11px] text-warning-foreground"
            >
              {{ t('notifications.settings.tokenWillClear') }}
            </p>
          </div>

          <div class="grid grid-cols-2 gap-2">
            <label class="block space-y-1 text-xs">
              <span class="font-medium text-muted-foreground">
                {{ t('notifications.settings.telegramTargetId') }}
              </span>
              <input
                v-model="tgChatId"
                data-testid="telegram-chat-id"
                type="text"
                placeholder="-100123456789"
                class="admin-input h-9 text-xs"
              />
            </label>

            <label class="block space-y-1 text-xs">
              <span class="font-medium text-muted-foreground">
                {{ t('notifications.settings.telegramThreadId') }}
              </span>
              <input
                v-model="tgThreadId"
                data-testid="telegram-thread-id"
                type="text"
                placeholder="42"
                class="admin-input h-9 text-xs"
              />
            </label>
          </div>

          <!-- Subscribed Topics -->
          <div class="space-y-1.5">
            <p class="text-xs font-medium text-muted-foreground">
              {{ t('notifications.settings.subscribedTopics') }}
            </p>
            <div class="grid grid-cols-2 gap-1.5">
              <label
                v-for="topic in ALL_NOTIFICATION_TOPICS"
                :key="`tg-${topic}`"
                class="flex cursor-pointer items-center gap-2 rounded-lg border bg-card px-2.5 py-1.5 text-[11px] text-foreground"
              >
                <input
                  :data-testid="`telegram-topic-${topic}`"
                  type="checkbox"
                  :checked="tgTopics.includes(topic)"
                  class="size-3.5 accent-primary"
                  @change="tgTopics = toggleTopic(tgTopics, topic)"
                />
                <span class="truncate">{{ topicLabel(topic) }}</span>
              </label>
            </div>
          </div>
        </div>

        <!-- Test Telegram Channel -->
        <div class="space-y-2 border-t pt-3">
          <Button
            type="button"
            variant="outline"
            size="sm"
            data-testid="telegram-test-btn"
            class="w-full"
            :disabled="tgTesting"
            @click="runChannelTest('telegram')"
          >
            <Send class="size-3.5" />
            <span>
              {{
                tgTesting
                  ? t('notifications.settings.testingChannel')
                  : t('notifications.settings.testChannel')
              }}
            </span>
          </Button>

          <div
            v-if="tgTestResult"
            data-testid="telegram-test-result"
            class="flex items-start gap-1.5 rounded-lg border p-2 text-xs"
            :class="
              tgTestResult.ok
                ? 'border-success/30 bg-success/5 text-success-foreground'
                : 'border-danger/30 bg-danger/5 text-danger-foreground'
            "
          >
            <CheckCircle2 v-if="tgTestResult.ok" class="mt-0.5 size-3.5 shrink-0" />
            <XCircle v-else class="mt-0.5 size-3.5 shrink-0" />
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
      <div class="flex flex-col justify-between rounded-xl border bg-background p-4 space-y-4">
        <div class="space-y-3">
          <div class="flex items-center justify-between gap-2">
            <div class="flex items-center gap-2">
              <MessageSquare class="size-4 text-studio-violet" />
              <h3 class="text-sm font-semibold text-foreground">
                {{ t('notifications.settings.zaloTitle') }}
              </h3>
            </div>
            <label class="inline-flex cursor-pointer items-center gap-2 text-xs font-medium">
              <span class="text-muted-foreground">{{ t('notifications.settings.channelEnabled') }}</span>
              <input
                v-model="zaloEnabled"
                data-testid="zalo-enabled-toggle"
                type="checkbox"
                class="size-4 accent-primary"
              />
            </label>
          </div>

          <!-- Bot Token with Patch<SecretValue> semantics -->
          <div class="space-y-1 text-xs">
            <div class="flex items-center justify-between gap-2">
              <span class="font-medium text-muted-foreground">
                {{ t('notifications.settings.botToken') }}
              </span>
              <button
                v-if="zaloHasSavedToken && !zaloClearToken"
                type="button"
                data-testid="zalo-clear-token"
                class="text-[11px] font-medium text-danger-foreground hover:underline"
                @click="zaloClearToken = true; zaloTokenInput = ''"
              >
                {{ t('notifications.settings.clearToken') }}
              </button>
              <button
                v-else-if="zaloClearToken"
                type="button"
                data-testid="zalo-undo-clear-token"
                class="text-[11px] font-medium text-primary hover:underline"
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
              class="admin-input h-9 text-xs"
              @input="zaloTokenInput && (zaloClearToken = false)"
            />
            <p
              v-if="zaloClearToken"
              data-testid="zalo-clear-notice"
              class="text-[11px] text-warning-foreground"
            >
              {{ t('notifications.settings.tokenWillClear') }}
            </p>
          </div>

          <label class="block space-y-1 text-xs">
            <span class="font-medium text-muted-foreground">
              {{ t('notifications.settings.zaloTargetId') }}
            </span>
            <input
              v-model="zaloRecipientId"
              data-testid="zalo-recipient-id"
              type="text"
              placeholder="zalo-user-or-group"
              class="admin-input h-9 text-xs"
            />
          </label>

          <!-- Subscribed Topics -->
          <div class="space-y-1.5">
            <p class="text-xs font-medium text-muted-foreground">
              {{ t('notifications.settings.subscribedTopics') }}
            </p>
            <div class="grid grid-cols-2 gap-1.5">
              <label
                v-for="topic in ALL_NOTIFICATION_TOPICS"
                :key="`zalo-${topic}`"
                class="flex cursor-pointer items-center gap-2 rounded-lg border bg-card px-2.5 py-1.5 text-[11px] text-foreground"
              >
                <input
                  :data-testid="`zalo-topic-${topic}`"
                  type="checkbox"
                  :checked="zaloTopics.includes(topic)"
                  class="size-3.5 accent-primary"
                  @change="zaloTopics = toggleTopic(zaloTopics, topic)"
                />
                <span class="truncate">{{ topicLabel(topic) }}</span>
              </label>
            </div>
          </div>
        </div>

        <!-- Test Zalo Channel -->
        <div class="space-y-2 border-t pt-3">
          <Button
            type="button"
            variant="outline"
            size="sm"
            data-testid="zalo-test-btn"
            class="w-full"
            :disabled="zaloTesting"
            @click="runChannelTest('zalo')"
          >
            <MessageSquare class="size-3.5" />
            <span>
              {{
                zaloTesting
                  ? t('notifications.settings.testingChannel')
                  : t('notifications.settings.testChannel')
              }}
            </span>
          </Button>

          <div
            v-if="zaloTestResult"
            data-testid="zalo-test-result"
            class="flex items-start gap-1.5 rounded-lg border p-2 text-xs"
            :class="
              zaloTestResult.ok
                ? 'border-success/30 bg-success/5 text-success-foreground'
                : 'border-danger/30 bg-danger/5 text-danger-foreground'
            "
          >
            <CheckCircle2 v-if="zaloTestResult.ok" class="mt-0.5 size-3.5 shrink-0" />
            <XCircle v-else class="mt-0.5 size-3.5 shrink-0" />
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
  </section>
</template>

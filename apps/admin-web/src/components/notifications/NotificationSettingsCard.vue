<script setup lang="ts">
import { Bell, Inbox, MessageSquare, RefreshCcw, Save, Send } from '@lucide/vue'
import { onMounted, ref } from 'vue'

import { isNotificationsDemoMode, notificationsApi } from '@/api/notifications'
import {
  type ExternalChannelKind,
  type NotificationSettings,
  type NotificationSettingsUpdate,
  type NotificationTopic,
  type TestChannelOutput,
} from '@/api/types/notifications'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

import NotificationChannelPanel from './NotificationChannelPanel.vue'

const { t } = useI18n()

const loading = ref(true)
const loaded = ref(false)
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
  loaded.value = true
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

function resolveBotTokenPatch(inputVal: string, clearFlag: boolean): string | null | undefined {
  if (clearFlag) return null
  const trimmed = inputVal.trim()
  if (trimmed.length > 0) return trimmed
  return undefined
}

async function saveSettings(): Promise<void> {
  if (saving.value || loading.value || !loaded.value) return
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
    if (tgTesting.value) return
    tgTesting.value = true
    tgTestResult.value = null
    try {
      const parsedThread = tgThreadId.value.trim() ? Number(tgThreadId.value.trim()) : null
      tgTestResult.value = await notificationsApi.testChannel('telegram', {
        chat_id: tgChatId.value.trim() || undefined,
        thread_id: Number.isFinite(parsedThread) ? parsedThread : null,
        bot_token: tgTokenInput.value.trim() || undefined,
      })
    } catch (error) {
      tgTestResult.value = { ok: false, channel, external_message_id: null, delivered_at_ms: Date.now(), error: error instanceof Error ? error.message : String(error) }
    } finally {
      tgTesting.value = false
    }
    return
  }

  if (zaloTesting.value) return
  zaloTesting.value = true
  zaloTestResult.value = null
  try {
    zaloTestResult.value = await notificationsApi.testChannel('zalo', {
      recipient_id: zaloRecipientId.value.trim() || undefined,
      bot_token: zaloTokenInput.value.trim() || undefined,
    })
  } catch (error) {
    zaloTestResult.value = { ok: false, channel, external_message_id: null, delivered_at_ms: Date.now(), error: error instanceof Error ? error.message : String(error) }
  } finally {
    zaloTesting.value = false
  }
}

onMounted(() => {
  void load()
})
</script>

<template>
  <section class="notification-workspace" data-testid="notification-settings-card" :aria-busy="loading">
    <p v-if="demoMode" data-testid="notification-settings-demo-banner" class="notification-demo">{{ t('notifications.settings.demoHint') }}</p>
    <p v-if="savedBanner" role="status" data-testid="notification-settings-saved" class="sys-success">{{ savedBanner }}</p>
    <div v-if="errorBanner" role="alert" class="sys-alert notification-load-error"><span>{{ errorBanner }}</span>
      <button v-if="!loaded" type="button" class="sys-button" :disabled="loading" @click="load">{{ t('common.retry') }}</button>
    </div>
    <p v-if="loading" role="status" class="sys-loading">{{ t('common.loading') }}</p>

    <div v-else-if="loaded" class="notification-content">
      <section class="notification-overview" :aria-label="t('system.detail.channelDraft')">
        <div class="notification-overview-origin"><Bell class="size-6" aria-hidden="true" /><span class="sys-kicker">VELA / SIGNALS</span></div>
        <div class="notification-overview-channels">
          <div><Inbox class="size-4" aria-hidden="true" /><span>Web UI</span><small>{{ t('system.detail.webAlwaysOn') }}</small></div>
          <div :data-enabled="tgEnabled"><Send class="size-4" aria-hidden="true" /><span>Telegram</span><small>{{ t(tgEnabled ? 'system.deck.enabled' : 'system.deck.disabled') }}</small></div>
          <div :data-enabled="zaloEnabled"><MessageSquare class="size-4" aria-hidden="true" /><span>Zalo</span><small>{{ t(zaloEnabled ? 'system.deck.enabled' : 'system.deck.disabled') }}</small></div>
        </div>
      </section>

      <section class="sys-panel notification-web">
        <header class="sys-panel-header"><span class="sys-section-number">01</span><div><h2>{{ t('system.detail.preferences') }}</h2><p>{{ t('system.detail.preferencesDescription') }}</p></div><Inbox class="sys-heading-icon size-5" aria-hidden="true" /></header>
        <fieldset class="notification-web-fields" :disabled="saving"><legend class="sr-only">{{ t('notifications.settings.webUiTitle') }}</legend>
          <label class="notification-toast"><div><span>{{ t('notifications.settings.toastEnabled') }}</span><p>{{ t('system.detail.webDescription') }}</p></div>
            <input v-model="toastEnabled" data-testid="webui-toast-toggle" type="checkbox" class="sys-toggle-input" />
          </label>
          <label class="sys-field"><span>{{ t('notifications.settings.retentionDays') }}</span><input v-model.number="retentionDays" data-testid="webui-retention-days" type="number" min="1" max="365" class="admin-input" /></label>
        </fieldset>
      </section>

      <header class="notification-section-heading"><span class="sys-section-number">02</span><div><h2>{{ t('system.detail.externalChannels') }}</h2><p>{{ t('system.detail.externalDescription') }}</p></div></header>
      <div class="notification-external-grid">
        <NotificationChannelPanel kind="telegram" v-model:enabled="tgEnabled" v-model:token="tgTokenInput" v-model:clear-token="tgClearToken"
          v-model:target="tgChatId" v-model:thread="tgThreadId" v-model:topics="tgTopics"
          :has-saved-token="tgHasSavedToken" :token-hint="tgTokenHint" :testing="tgTesting" :test-result="tgTestResult" :disabled="saving" @test="runChannelTest('telegram')" />
        <NotificationChannelPanel kind="zalo" v-model:enabled="zaloEnabled" v-model:token="zaloTokenInput" v-model:clear-token="zaloClearToken"
          v-model:target="zaloRecipientId" v-model:topics="zaloTopics"
          :has-saved-token="zaloHasSavedToken" :token-hint="zaloTokenHint" :testing="zaloTesting" :test-result="zaloTestResult" :disabled="saving" @test="runChannelTest('zalo')" />
      </div>
    </div>
    <footer class="notification-savebar">
      <span>{{ loaded ? t('system.detail.configurationRevision', { revision }) : t('common.loading') }}</span>
      <div class="sys-actions">
        <button type="button" class="sys-button" :disabled="saving || loading" @click="load"><RefreshCcw class="size-3.5" aria-hidden="true" />{{ t('system.detail.reloadSettings') }}</button>
        <Button type="button" data-testid="notification-settings-save" class="sys-primary" :disabled="saving || loading || !loaded || tgTesting || zaloTesting" @click="saveSettings">
          <Save class="size-3.5" aria-hidden="true" />{{ t(saving ? 'notifications.settings.saving' : 'notifications.settings.save') }}
        </Button>
      </div>
    </footer>
  </section>
</template>

<style src="./notification-settings.css"></style>

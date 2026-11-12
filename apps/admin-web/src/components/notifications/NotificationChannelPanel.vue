<script setup lang="ts">
import { CheckCircle2, MessageSquare, Send, XCircle } from '@lucide/vue'
import { computed, useId } from 'vue'

import { ALL_NOTIFICATION_TOPICS, type ExternalChannelKind, type NotificationTopic, type TestChannelOutput } from '@/api/types/notifications'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { MessageKey } from '@/i18n/messages'

const props = defineProps<{
  kind: ExternalChannelKind
  hasSavedToken: boolean
  tokenHint: string | null
  testing: boolean
  testResult: TestChannelOutput | null
  disabled: boolean
}>()
const enabled = defineModel<boolean>('enabled', { required: true })
const token = defineModel<string>('token', { required: true })
const clearToken = defineModel<boolean>('clearToken', { required: true })
const target = defineModel<string>('target', { required: true })
const thread = defineModel<string>('thread', { default: '' })
const topics = defineModel<NotificationTopic[]>('topics', { required: true })
const emit = defineEmits<{ test: [] }>()
const { t } = useI18n()
const tokenId = useId()
const title = computed(() => t(props.kind === 'telegram' ? 'notifications.settings.telegramTitle' : 'notifications.settings.zaloTitle'))
const icon = computed(() => props.kind === 'telegram' ? Send : MessageSquare)
function topicLabel(topic: NotificationTopic) { return t(`notifications.topic.${topic}` as MessageKey) }
function toggleTopic(topic: NotificationTopic) {
  topics.value = topics.value.includes(topic) ? topics.value.filter(item => item !== topic) : [...topics.value, topic]
}
</script>

<template>
  <section class="sys-panel notification-channel" :data-channel="kind">
    <header class="notification-channel-heading">
      <span class="notification-channel-icon"><component :is="icon" class="size-5" aria-hidden="true" /></span>
      <div><h3>{{ title }}</h3><p>{{ t(kind === 'telegram' ? 'system.detail.telegramDescription' : 'system.detail.zaloDescription') }}</p></div>
      <label class="notification-channel-toggle">
        <span class="sr-only">{{ title }} · {{ t('notifications.settings.channelEnabled') }}</span>
        <input v-model="enabled" :data-testid="`${kind}-enabled-toggle`" type="checkbox" class="sys-toggle-input" :disabled="disabled" />
      </label>
    </header>
    <fieldset class="notification-channel-fields" :disabled="disabled">
      <legend class="sr-only">{{ title }} · {{ t('system.detail.channelSettings') }}</legend>
      <div class="sys-field">
        <div class="notification-token-heading"><label :for="tokenId">{{ t('notifications.settings.botToken') }}</label>
          <button v-if="hasSavedToken && !clearToken" type="button" :data-testid="`${kind}-clear-token`" class="notification-clear-token" @click="clearToken = true; token = ''">{{ t('notifications.settings.clearToken') }}</button>
          <button v-else-if="clearToken" type="button" :data-testid="`${kind}-undo-clear-token`" class="sys-text-button" @click="clearToken = false">{{ t('notifications.settings.undoClearToken') }}</button>
        </div>
        <input :id="tokenId" v-model="token" :data-testid="`${kind}-bot-token`" type="password" autocomplete="off" class="admin-input"
          :placeholder="hasSavedToken && !clearToken ? t('notifications.settings.botTokenKeepPlaceholder', { hint: tokenHint ?? '••••' }) : t('notifications.settings.botTokenNewPlaceholder')"
          @input="token && (clearToken = false)" />
        <p v-if="clearToken" :data-testid="`${kind}-clear-notice`" class="notification-token-warning">{{ t('notifications.settings.tokenWillClear') }}</p>
      </div>
      <div :class="kind === 'telegram' ? 'sys-form-grid' : ''">
        <label class="sys-field"><span>{{ t(kind === 'telegram' ? 'notifications.settings.telegramTargetId' : 'notifications.settings.zaloTargetId') }}</span>
          <input v-model="target" :data-testid="kind === 'telegram' ? 'telegram-chat-id' : 'zalo-recipient-id'" type="text" class="admin-input" :placeholder="kind === 'telegram' ? '-100123456789' : 'zalo-user-or-group'" />
        </label>
        <label v-if="kind === 'telegram'" class="sys-field"><span>{{ t('notifications.settings.telegramThreadId') }}</span>
          <input v-model="thread" data-testid="telegram-thread-id" type="text" inputmode="numeric" class="admin-input" placeholder="42" />
        </label>
      </div>
      <fieldset class="notification-topic-fieldset">
        <legend>{{ t('system.detail.channelTopics') }}<span>{{ topics.length }} / {{ ALL_NOTIFICATION_TOPICS.length }}</span></legend>
        <div class="notification-topics">
          <label v-for="topic in ALL_NOTIFICATION_TOPICS" :key="topic" :data-selected="topics.includes(topic)">
            <input :data-testid="`${kind}-topic-${topic}`" type="checkbox" :checked="topics.includes(topic)" @change="toggleTopic(topic)" />
            <span>{{ topicLabel(topic) }}</span>
          </label>
        </div>
      </fieldset>
    </fieldset>
    <footer class="notification-test">
      <Button type="button" variant="outline" :data-testid="`${kind}-test-btn`" :disabled="disabled || testing" @click="emit('test')">
        <component :is="icon" class="size-3.5" aria-hidden="true" />{{ t(testing ? 'notifications.settings.testingChannel' : 'notifications.settings.testChannel') }}
      </Button>
      <div v-if="testResult" :data-testid="`${kind}-test-result`" role="status" :class="testResult.ok ? 'sys-success' : 'sys-alert'">
        <component :is="testResult.ok ? CheckCircle2 : XCircle" class="size-4 shrink-0" aria-hidden="true" />
        <span>{{ testResult.ok ? `OK (${testResult.external_message_id ?? '—'})` : testResult.error }}</span>
      </div>
      <p>{{ t('system.detail.deliveryHint') }}</p>
    </footer>
  </section>
</template>

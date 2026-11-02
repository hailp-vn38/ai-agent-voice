<script setup lang="ts">
import { ref } from 'vue'
import { RotateCcw, Send, Square } from '@lucide/vue'
import { renderMarkdown } from '@/lib/markdown'
import { useI18n } from '@/composables/useI18n'
import type { WikiChatMessage } from './useWikiChat'

defineProps<{
  messages: WikiChatMessage[]
  busy: boolean
  drawerOpen?: boolean
}>()

const emit = defineEmits<{
  send: [text: string]
  stop: []
  reset: []
  retry: [id: string]
  showGraph: [id: string]
}>()

const { t } = useI18n()
const draft = ref('')

function submit() {
  const text = draft.value.trim()
  if (!text) return
  emit('send', text)
  draft.value = ''
}

function onAnswerClick(event: MouseEvent) {
  const anchor = (event.target as HTMLElement | null)?.closest<HTMLAnchorElement>('a.knowledge-wikilink')
  if (anchor) event.preventDefault()
}

function errorText(code: string | null): string {
  if (code === 'wiki_chat_not_configured') return t('knowledgeV3.chatNotConfigured')
  if (code === 'wiki_chat_busy') return t('knowledgeV3.chatBusy')
  if (code === 'wiki_chat_connection_lost') return t('knowledgeV3.chatConnectionLost')
  return t('knowledgeV3.chatFailed')
}
</script>

<template>
  <aside id="wiki-chat-panel" class="wiki-chat-panel" :class="{ 'is-drawer-open': drawerOpen }" :aria-label="t('knowledgeV3.chatTitle')">
    <header class="wiki-chat-header">
      <div>
        <h2>{{ t('knowledgeV3.chatTitle') }}</h2>
        <p>{{ t('knowledgeV3.chatDescription') }}</p>
      </div>
      <button type="button" class="wiki-chat-icon-button" :aria-label="t('knowledgeV3.chatClear')"
        :title="t('knowledgeV3.chatClear')" :disabled="busy && messages.length === 0" @click="emit('reset')">
        <RotateCcw :size="15" aria-hidden="true" />
      </button>
    </header>

    <div class="wiki-chat-messages" :aria-label="t('knowledgeV3.chatConversation')">
      <p v-if="messages.length === 0" class="wiki-chat-empty">{{ t('knowledgeV3.chatEmpty') }}</p>
      <article v-for="message in messages" :key="message.id" class="wiki-chat-message"
        :class="['is-' + message.role, 'is-' + message.status]" :data-message-id="message.id">
        <div class="wiki-chat-message-label">{{ message.role === 'user' ? t('knowledgeV3.chatYou') : t('knowledgeV3.chatAssistant') }}</div>
        <div v-if="message.role === 'user'" class="wiki-chat-user-text">{{ message.text }}</div>
        <div v-else class="wiki-chat-answer" :aria-live="message.status === 'done' ? 'polite' : 'off'"
          @click="onAnswerClick" v-html="renderMarkdown(message.text || (message.status === 'streaming' ? t('knowledgeV3.chatThinking') : ''))"></div>

        <ul v-if="message.tools.length" class="wiki-chat-tools" :aria-label="t('knowledgeV3.chatToolActivity')">
          <li v-for="tool in message.tools" :key="tool.call_id" :class="'is-' + tool.state">
            <span class="wiki-chat-tool-state" aria-hidden="true">{{ tool.state === 'completed' ? '✓' : tool.state === 'failed' ? '!' : '…' }}</span>
            <span>{{ tool.name }}</span>
            <span v-if="tool.query" class="wiki-chat-tool-query">{{ tool.query }}</span>
          </li>
        </ul>

        <p v-if="message.status === 'stopped'" class="wiki-chat-status">{{ t('knowledgeV3.chatStopped') }}</p>
        <div v-if="message.status === 'failed'" class="wiki-chat-error" role="alert">
          <span>{{ errorText(message.errorCode) }}</span>
          <a v-if="message.errorCode === 'wiki_chat_not_configured'" href="/jobs/settings/providers">
            {{ t('knowledgeV3.chatOpenSettings') }}
          </a>
          <button v-else type="button" @click="emit('retry', message.id)">{{ t('knowledgeV3.chatRetry') }}</button>
        </div>
        <button v-if="message.graph?.nodes.length" type="button" class="wiki-chat-show-graph"
          @click="emit('showGraph', message.id)">{{ t('knowledgeV3.chatShowGraph') }}</button>
      </article>
    </div>

    <form class="wiki-chat-compose" @submit.prevent="submit">
      <label class="sr-only" for="wiki-chat-question">{{ t('knowledgeV3.chatInputLabel') }}</label>
      <textarea id="wiki-chat-question" v-model="draft" rows="3" :placeholder="t('knowledgeV3.chatPlaceholder')"
        :disabled="busy" @keydown.enter.exact.prevent="submit"></textarea>
      <div class="wiki-chat-compose-actions">
        <button v-if="busy" type="button" class="wiki-chat-stop" @click="emit('stop')">
          <Square :size="13" aria-hidden="true" /> {{ t('knowledgeV3.chatStop') }}
        </button>
        <button v-else type="submit" class="wiki-chat-send" :disabled="!draft.trim()">
          <Send :size="13" aria-hidden="true" /> {{ t('knowledgeV3.chatSend') }}
        </button>
      </div>
    </form>
  </aside>
</template>

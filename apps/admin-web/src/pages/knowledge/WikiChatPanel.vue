<script setup lang="ts">
import { nextTick, onUpdated, ref, watch } from 'vue'
import { RotateCcw, Send, Square } from '@lucide/vue'
import { renderMarkdown } from '@/lib/markdown'
import { useI18n } from '@/composables/useI18n'
import type { WikiChatMessage } from './useWikiChat'

const props = defineProps<{
  messages: WikiChatMessage[]
  busy: boolean
  drawerOpen?: boolean
  suggestions?: Array<{ label: string; prompt: string }>
  highlightedNode?: string | null
}>()

const emit = defineEmits<{
  send: [text: string]
  stop: []
  reset: []
  retry: [id: string]
  showGraph: [id: string]
  focusCitation: [path: string]
  highlightCitation: [path: string | null]
}>()

const { t } = useI18n()
const draft = ref('')
const panelElement = ref<HTMLElement | null>(null)

function submit() {
  const text = draft.value.trim()
  if (!text) return
  emit('send', text)
  draft.value = ''
}

function citationPath(target: string, citations: string[]): string | null {
  const normalized = target.trim().replace(/^qmd:\/\//, '').split('#', 1)[0]?.trim() ?? ''
  if (!normalized) return null
  const candidates = normalized.startsWith('wiki/') || normalized.startsWith('sources/')
    ? [normalized]
    : [`wiki/${normalized}`]
  for (const candidate of candidates) {
    if (citations.includes(candidate)) return candidate
  }
  const suffixMatches = citations.filter(path => path.endsWith(`/${normalized}`))
  return suffixMatches.length === 1 ? suffixMatches[0]! : null
}

function renderedAnswer(message: WikiChatMessage): string {
  const template = document.createElement('template')
  template.innerHTML = renderMarkdown(message.text || (message.status === 'streaming' ? t('knowledgeV3.chatThinking') : ''))
  template.content.querySelectorAll<HTMLAnchorElement>('a.knowledge-wikilink').forEach(anchor => {
    const path = citationPath(anchor.dataset.wikilink ?? '', message.citations)
    if (!path) {
      anchor.replaceWith(document.createTextNode(anchor.textContent ?? ''))
      return
    }
    const index = message.citations.indexOf(path)
    anchor.className = 'wiki-chat-citation'
    anchor.href = '#'
    anchor.dataset.citationPath = path
    anchor.textContent = `[${index + 1}]`
  })
  return template.innerHTML
}

function syncCitationHighlight() {
  const highlighted = props.highlightedNode
  panelElement.value?.querySelectorAll<HTMLAnchorElement>('a.wiki-chat-citation').forEach(anchor => {
    anchor.classList.toggle('is-highlighted', Boolean(highlighted && anchor.dataset.citationPath === highlighted))
  })
}

watch(() => props.highlightedNode, () => { void nextTick(syncCitationHighlight) })
onUpdated(syncCitationHighlight)

function onAnswerClick(event: MouseEvent) {
  const anchor = (event.target as HTMLElement | null)?.closest<HTMLAnchorElement>('a.wiki-chat-citation')
  if (!anchor) return
  event.preventDefault()
  const path = anchor.dataset.citationPath
  if (path) emit('focusCitation', path)
}

function onAnswerMouseOver(event: MouseEvent) {
  const anchor = (event.target as HTMLElement | null)?.closest<HTMLAnchorElement>('a.wiki-chat-citation')
  if (anchor?.dataset.citationPath) emit('highlightCitation', anchor.dataset.citationPath)
}

function onAnswerMouseOut(event: MouseEvent) {
  const anchor = (event.target as HTMLElement | null)?.closest<HTMLAnchorElement>('a.wiki-chat-citation')
  const related = event.relatedTarget as Node | null
  if (anchor?.dataset.citationPath && (!related || !anchor.contains(related))) emit('highlightCitation', null)
}

function sourceTitle(message: WikiChatMessage, path: string): string {
  return message.graph?.nodes.find(node => node.path === path || node.id === path)?.title ?? path
}

function errorText(code: string | null): string {
  if (code === 'wiki_chat_not_configured') return t('knowledgeV3.chatNotConfigured')
  if (code === 'wiki_chat_busy') return t('knowledgeV3.chatBusy')
  if (code === 'wiki_chat_connection_lost') return t('knowledgeV3.chatConnectionLost')
  return t('knowledgeV3.chatFailed')
}
</script>

<template>
  <aside id="wiki-chat-panel" ref="panelElement" class="wiki-chat-panel" :class="{ 'is-drawer-open': drawerOpen }" :aria-label="t('knowledgeV3.chatTitle')">
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
      <template v-if="messages.length === 0">
        <p class="wiki-chat-empty">{{ t('knowledgeV3.chatEmpty') }}</p>
        <div v-if="suggestions?.length" class="wiki-chat-suggestions" :aria-label="t('knowledgeV3.chatSuggestions')">
          <button v-for="suggestion in suggestions" :key="suggestion.label" type="button" class="wiki-chat-suggestion"
            @click="emit('send', suggestion.prompt)">{{ suggestion.label }}</button>
        </div>
      </template>
      <article v-for="message in messages" :key="message.id" class="wiki-chat-message"
        :class="['is-' + message.role, 'is-' + message.status]" :data-message-id="message.id">
        <div class="wiki-chat-message-label">{{ message.role === 'user' ? t('knowledgeV3.chatYou') : t('knowledgeV3.chatAssistant') }}</div>
        <div v-if="message.role === 'user'" class="wiki-chat-user-text">{{ message.text }}</div>
        <div v-else class="wiki-chat-answer" :aria-live="message.status === 'done' ? 'polite' : 'off'"
          @click="onAnswerClick" @mouseover="onAnswerMouseOver" @mouseout="onAnswerMouseOut"
          v-html="renderedAnswer(message)"></div>

        <ul v-if="message.tools.some(tool => tool.state !== 'completed')" class="wiki-chat-tools" :aria-label="t('knowledgeV3.chatToolActivity')">
          <li v-for="tool in message.tools.filter(item => item.state !== 'completed')" :key="tool.call_id" :class="'is-' + tool.state">
            <span class="wiki-chat-tool-state" aria-hidden="true">{{ tool.state === 'completed' ? '✓' : tool.state === 'failed' ? '!' : '…' }}</span>
            <span>{{ tool.name }}</span>
            <span v-if="tool.query" class="wiki-chat-tool-query">{{ tool.query }}</span>
          </li>
        </ul>
        <details v-for="tool in message.tools.filter(item => item.state === 'completed')" :key="tool.call_id" class="wiki-chat-tool-summary">
          <summary><span>{{ tool.name }}</span><span v-if="tool.query" class="wiki-chat-tool-query">{{ tool.query }}</span></summary>
          <p v-if="tool.query" class="wiki-chat-tool-detail-query">{{ tool.query }}</p>
          <ul v-if="tool.pages.length" class="wiki-chat-tool-pages">
            <li v-for="path in tool.pages" :key="path">{{ path }}</li>
          </ul>
        </details>

        <ol v-if="message.citations.length" class="wiki-chat-sources" :aria-label="t('knowledgeV3.chatSources')">
          <li v-for="(path, index) in message.citations" :key="path"
            :class="{ 'is-highlighted': highlightedNode === path }"
            @mouseenter="emit('highlightCitation', path)" @mouseleave="emit('highlightCitation', null)">
            <button type="button" class="wiki-chat-source-link" @click="emit('focusCitation', path)">
              <span>[{{ index + 1 }}]</span> {{ sourceTitle(message, path) }}
            </button>
          </li>
        </ol>

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

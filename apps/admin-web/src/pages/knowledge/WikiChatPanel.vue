<script setup lang="ts">
import { computed, nextTick, onMounted, onUpdated, ref, watch } from 'vue'
import { AlertCircle, ArrowUp, ArrowUpRight, BookOpenText, ChevronRight, CircleStop, FileText, Files, LoaderCircle, Network, Plus, RotateCcw, Search, Sparkles, Square, Wrench, X } from '@lucide/vue'
import { renderMarkdown } from '@/lib/markdown'
import { useI18n } from '@/composables/useI18n'
import { useTheme } from '@/composables/useTheme'
import { renderMermaidBlocks } from './mermaidRenderer'
import { MAX_PINNED_FILES, type WikiChatFile, type WikiChatMessage } from './useWikiChat'

const props = defineProps<{
  messages: WikiChatMessage[]
  busy: boolean
  drawerOpen?: boolean
  suggestions?: Array<{ label: string; prompt: string }>
  highlightedNode?: string | null
  pinnedFiles?: WikiChatFile[]
  openFile?: WikiChatFile | null
}>()

const emit = defineEmits<{
  send: [text: string]
  stop: []
  reset: []
  retry: [id: string]
  showGraph: [id: string]
  focusCitation: [path: string]
  highlightCitation: [path: string | null]
  pinFile: [file: WikiChatFile]
  unpinFile: [path: string]
}>()

const { t } = useI18n()
const { dark } = useTheme()
const draft = ref('')
const panelElement = ref<HTMLElement | null>(null)
const messagesElement = ref<HTMLElement | null>(null)
const questionInput = ref<HTMLTextAreaElement | null>(null)
const pinnedFileList = computed(() => props.pinnedFiles ?? [])
const pinnableFile = computed(() => {
  const file = props.openFile
  return file && !pinnedFileList.value.some(pinned => pinned.path === file.path) ? file : null
})
const pinLimitReached = computed(() => pinnedFileList.value.length >= MAX_PINNED_FILES)
// How close to the end still counts as following the answer as it streams.
const FOLLOW_DISTANCE = 72
let followingAnswer = true

function onMessagesScroll() {
  const element = messagesElement.value
  if (element) followingAnswer = element.scrollHeight - element.scrollTop - element.clientHeight < FOLLOW_DISTANCE
}
function followAnswer() {
  const element = messagesElement.value
  if (element && followingAnswer) element.scrollTop = element.scrollHeight
}
watch(() => props.messages.map(message => message.text.length + message.tools.length).join(), followAnswer, { flush: 'post' })
watch(() => props.messages.length, () => { followingAnswer = true }, { flush: 'pre' })

watch(draft, () => {
  const input = questionInput.value
  if (!input) return
  input.style.height = 'auto'
  input.style.height = input.scrollHeight + 'px'
}, { flush: 'post' })

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
  template.innerHTML = renderMarkdown(message.text)
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
    anchor.textContent = String(index + 1)
    anchor.title = sourceTitle(message, path)
    anchor.setAttribute('aria-label', t('knowledgeV3.chatSources') + ' ' + (index + 1) + ': ' + anchor.title)
  })
  return template.innerHTML
}

// Stays empty while streaming, so screen readers hear each answer once, when complete.
const completedAnnouncement = computed(() => {
  const latest = props.messages[props.messages.length - 1]
  if (latest?.role !== 'assistant' || latest.status !== 'done') return ''
  const template = document.createElement('template')
  template.innerHTML = renderedAnswer(latest)
  template.content.querySelectorAll('pre.knowledge-mermaid').forEach(block => block.remove())
  return template.content.textContent?.trim() ?? ''
})

function syncCitationHighlight() {
  const highlighted = props.highlightedNode
  panelElement.value?.querySelectorAll<HTMLAnchorElement>('a.wiki-chat-citation').forEach(anchor => {
    anchor.classList.toggle('is-highlighted', Boolean(highlighted && anchor.dataset.citationPath === highlighted))
  })
}

// A streaming answer replaces its HTML on every delta, so diagrams wait for it to finish.
function renderDiagrams() {
  panelElement.value?.querySelectorAll<HTMLElement>('.wiki-chat-message:not(.is-streaming) .wiki-chat-answer').forEach(answer => {
    if (answer.querySelector('pre.knowledge-mermaid[data-mermaid-source="true"]')) {
      void renderMermaidBlocks(answer, dark.value, true).then(followAnswer)
    }
  })
}

watch(() => props.highlightedNode, () => { void nextTick(syncCitationHighlight) })
onMounted(renderDiagrams)
onUpdated(() => {
  syncCitationHighlight()
  renderDiagrams()
})

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

function toolIcon(name: string) {
  if (name === 'wiki_query') return Search
  if (name === 'wiki_get') return BookOpenText
  if (name === 'wiki_multi_get') return Files
  return Wrench
}

function toolLabel(name: string): string {
  if (name === 'wiki_query') return t('knowledgeV3.chatToolSearch')
  if (name === 'wiki_get') return t('knowledgeV3.chatToolRead')
  if (name === 'wiki_multi_get') return t('knowledgeV3.chatToolReadMany')
  return name
}

function completedTools(message: WikiChatMessage) {
  return message.tools.filter(tool => tool.state === 'completed')
}

function activeTools(message: WikiChatMessage) {
  return message.tools.filter(tool => tool.state !== 'completed')
}

function errorText(code: string | null): string {
  if (code === 'wiki_chat_not_configured') return t('knowledgeV3.chatNotConfigured')
  if (code === 'wiki_chat_busy') return t('knowledgeV3.chatBusy')
  if (code === 'wiki_conversation_full') return t('knowledgeV3.chatConversationFull')
  if (code === 'wiki_chat_connection_lost') return t('knowledgeV3.chatConnectionLost')
  return t('knowledgeV3.chatFailed')
}
</script>

<template>
  <aside id="wiki-chat-panel" ref="panelElement" class="wiki-chat-panel" :class="{ 'is-drawer-open': drawerOpen }" :aria-label="t('knowledgeV3.chatTitle')">
    <header class="wiki-chat-header">
      <slot name="title">
        <span class="wiki-chat-orb wiki-chat-orb-small" aria-hidden="true"><Sparkles :size="11" /></span>
        <h2>{{ t('knowledgeV3.chatTitle') }}</h2>
      </slot>
      <button v-if="messages.length" type="button" class="wiki-chat-icon-button" :aria-label="t('knowledgeV3.chatClear')"
        :title="t('knowledgeV3.chatClear')" @click="emit('reset')">
        <RotateCcw :size="14" aria-hidden="true" />
      </button>
    </header>

    <div class="wiki-chat-announcement sr-only" role="status" aria-live="polite" aria-atomic="true">{{ completedAnnouncement }}</div>
    <div ref="messagesElement" class="wiki-chat-messages" :aria-label="t('knowledgeV3.chatConversation')" @scroll.passive="onMessagesScroll">
      <div v-if="messages.length === 0" class="wiki-chat-welcome">
        <span class="wiki-chat-orb" aria-hidden="true"><Sparkles :size="22" /></span>
        <p class="wiki-chat-empty">{{ t('knowledgeV3.chatEmpty') }}</p>
        <p class="wiki-chat-empty-note">{{ t('knowledgeV3.chatDescription') }}</p>
        <div v-if="suggestions?.length" class="wiki-chat-suggestions" :aria-label="t('knowledgeV3.chatSuggestions')">
          <button v-for="suggestion in suggestions" :key="suggestion.label" type="button" class="wiki-chat-suggestion"
            :title="suggestion.prompt" @click="emit('send', suggestion.prompt)">
            <span>{{ suggestion.label }}</span><ArrowUpRight :size="13" aria-hidden="true" />
          </button>
        </div>
      </div>
      <article v-for="message in messages" :key="message.id" class="wiki-chat-message"
        :class="['is-' + message.role, 'is-' + message.status]" :data-message-id="message.id"
        :aria-label="message.role === 'user' ? t('knowledgeV3.chatYou') : t('knowledgeV3.chatAssistant')">
        <ul v-if="message.role === 'user' && message.files?.length" class="wiki-chat-user-files" :aria-label="t('knowledgeV3.chatPinnedFiles')">
          <li v-for="file in message.files" :key="file.path" class="wiki-chat-file" :title="file.path">
            <FileText :size="12" aria-hidden="true" /><span class="wiki-chat-file-name">{{ file.title }}</span>
          </li>
        </ul>
        <div v-if="message.role === 'user'" class="wiki-chat-user-text">{{ message.text }}</div>
        <template v-else>
          <details v-if="completedTools(message).length" class="wiki-chat-trail">
            <summary>
              <ChevronRight :size="12" class="wiki-chat-trail-chevron" aria-hidden="true" />
              <span>{{ t('knowledgeV3.chatTrail', { count: completedTools(message).length }) }}</span>
            </summary>
            <div class="wiki-chat-trail-steps">
              <details v-for="tool in completedTools(message)" :key="tool.call_id" class="wiki-chat-tool-summary">
                <summary>
                  <component :is="toolIcon(tool.name)" :size="12" aria-hidden="true" />
                  <span class="wiki-chat-tool-name">{{ toolLabel(tool.name) }}</span>
                  <span v-if="tool.query" class="wiki-chat-tool-query">{{ tool.query }}</span>
                </summary>
                <p v-if="tool.query" class="wiki-chat-tool-detail-query">{{ tool.query }}</p>
                <ul v-if="tool.pages.length" class="wiki-chat-tool-pages">
                  <li v-for="path in tool.pages" :key="path">{{ path }}</li>
                </ul>
              </details>
            </div>
          </details>

          <ul v-if="activeTools(message).length" class="wiki-chat-tools" :aria-label="t('knowledgeV3.chatToolActivity')">
            <li v-for="tool in activeTools(message)" :key="tool.call_id" :class="'is-' + tool.state">
              <AlertCircle v-if="tool.state === 'failed'" :size="12" aria-hidden="true" />
              <LoaderCircle v-else :size="12" class="wiki-chat-spinner" aria-hidden="true" />
              <span class="wiki-chat-tool-name">{{ toolLabel(tool.name) }}</span>
              <span v-if="tool.query" class="wiki-chat-tool-query">{{ tool.query }}</span>
            </li>
          </ul>

          <div v-if="message.status === 'streaming' && !message.text && !activeTools(message).length" class="wiki-chat-thinking">
            <span class="wiki-chat-thinking-dots" aria-hidden="true"><i></i><i></i><i></i></span>
            <span>{{ t('knowledgeV3.chatThinking') }}</span>
          </div>
          <div v-if="message.text" :key="dark ? 'dark' : 'light'" class="wiki-chat-answer"
            @click="onAnswerClick" @mouseover="onAnswerMouseOver" @mouseout="onAnswerMouseOut"
            v-html="renderedAnswer(message)"></div>

          <p v-if="message.contentTruncated" class="wiki-chat-status">
            <AlertCircle :size="12" aria-hidden="true" /> {{ t('knowledgeV3.chatAnswerTruncated') }}
          </p>
          <p v-if="message.saved === false" class="wiki-chat-error" role="alert">
            <AlertCircle :size="14" class="shrink-0" aria-hidden="true" />
            <span>{{ t('knowledgeV3.chatSaveFailed') }}</span>
          </p>

          <ol v-if="message.citations.length" class="wiki-chat-sources" :aria-label="t('knowledgeV3.chatSources')">
            <li v-for="(path, index) in message.citations" :key="path"
              :class="{ 'is-highlighted': highlightedNode === path }"
              @mouseenter="emit('highlightCitation', path)" @mouseleave="emit('highlightCitation', null)">
              <button type="button" class="wiki-chat-source-link" :title="path" @click="emit('focusCitation', path)">
                <span class="wiki-chat-source-number">{{ index + 1 }}</span>
                <span class="wiki-chat-source-title">{{ sourceTitle(message, path) }}</span>
              </button>
            </li>
          </ol>

          <p v-if="message.status === 'stopped'" class="wiki-chat-status">
            <CircleStop :size="12" aria-hidden="true" /> {{ t('knowledgeV3.chatStopped') }}
          </p>
          <div v-if="message.status === 'failed'" class="wiki-chat-error" role="alert">
            <AlertCircle :size="14" class="shrink-0" aria-hidden="true" />
            <span>{{ errorText(message.errorCode) }}</span>
            <a v-if="message.errorCode === 'wiki_chat_not_configured'" href="/jobs/settings/providers">
              {{ t('knowledgeV3.chatOpenSettings') }}
            </a>
            <button v-else-if="message.errorCode !== 'wiki_conversation_full'" type="button" @click="emit('retry', message.id)">{{ t('knowledgeV3.chatRetry') }}</button>
          </div>
          <button v-if="message.graph?.nodes.length" type="button" class="wiki-chat-show-graph"
            @click="emit('showGraph', message.id)">
            <Network :size="12" aria-hidden="true" /> {{ t('knowledgeV3.chatShowGraph') }}
          </button>
        </template>
      </article>
    </div>

    <form class="wiki-chat-compose" @submit.prevent="submit">
      <div v-if="pinnedFileList.length || pinnableFile" class="wiki-chat-files" role="group" :aria-label="t('knowledgeV3.chatPinnedFiles')">
        <span v-for="file in pinnedFileList" :key="file.path" class="wiki-chat-file" :title="file.path">
          <FileText :size="12" aria-hidden="true" /><span class="wiki-chat-file-name">{{ file.title }}</span>
          <button type="button" class="wiki-chat-file-remove" :aria-label="t('knowledgeV3.chatUnpinFile', { title: file.title })"
            :title="t('knowledgeV3.chatUnpinFile', { title: file.title })" @click="emit('unpinFile', file.path)">
            <X :size="11" aria-hidden="true" />
          </button>
        </span>
        <button v-if="pinnableFile" type="button" class="wiki-chat-file-add" :disabled="pinLimitReached"
          :title="pinLimitReached ? t('knowledgeV3.chatPinLimit', { count: MAX_PINNED_FILES }) : t('knowledgeV3.chatPinFile', { title: pinnableFile.title })"
          :aria-label="t('knowledgeV3.chatPinFile', { title: pinnableFile.title })" @click="emit('pinFile', pinnableFile)">
          <Plus :size="12" aria-hidden="true" /><span class="wiki-chat-file-name">{{ pinnableFile.title }}</span>
        </button>
      </div>
      <label class="sr-only" for="wiki-chat-question">{{ t('knowledgeV3.chatInputLabel') }}</label>
      <div class="wiki-chat-compose-box">
        <textarea id="wiki-chat-question" ref="questionInput" v-model="draft" rows="1" :placeholder="t('knowledgeV3.chatPlaceholder')"
          :disabled="busy" @keydown.enter.exact.prevent="submit"></textarea>
        <button v-if="busy" type="button" class="wiki-chat-stop" :aria-label="t('knowledgeV3.chatStop')"
          :title="t('knowledgeV3.chatStop')" @click="emit('stop')">
          <Square :size="12" fill="currentColor" aria-hidden="true" />
        </button>
        <button v-else type="submit" class="wiki-chat-send" :aria-label="t('knowledgeV3.chatSend')"
          :title="t('knowledgeV3.chatSend')" :disabled="!draft.trim()">
          <ArrowUp :size="15" aria-hidden="true" />
        </button>
      </div>
    </form>
  </aside>
</template>

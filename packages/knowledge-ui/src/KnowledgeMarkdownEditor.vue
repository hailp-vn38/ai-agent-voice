<script lang="ts">
export type EditorMode = 'write' | 'split' | 'preview'
export type Edit = { text: string; start: number; end: number; selStart: number; selEnd: number }

const ORDERED = /^(\s*)(\d+)\.\s/
const LIST = /^(\s*)([-*+]|\d+\.)\s(\[[ xX]\]\s)?/

/** The edit that wraps the selection in `before`/`after`, or unwraps it when it is already wrapped. */
export function wrapEdit(value: string, start: number, end: number, before: string, after = before, placeholder = ''): Edit {
  const selected = value.slice(start, end)
  if (value.slice(start - before.length, start) === before && value.slice(end, end + after.length) === after) {
    return { text: selected, start: start - before.length, end: end + after.length,
      selStart: start - before.length, selEnd: end - before.length }
  }
  const inner = selected || placeholder
  return { text: before + inner + after, start, end,
    selStart: start + before.length, selEnd: start + before.length + inner.length }
}

/** The edit that toggles `prefix` on every line the selection touches. Headings replace each other. */
export function linePrefixEdit(value: string, start: number, end: number, prefix: string): Edit {
  const lineStart = value.lastIndexOf('\n', start - 1) + 1
  const nextBreak = value.indexOf('\n', end)
  const lineEnd = nextBreak < 0 ? value.length : nextBreak
  const lines = value.slice(lineStart, lineEnd).split('\n')
  const strip = (line: string) => prefix.startsWith('#') ? line.replace(/^#{1,6}\s/, '') : line
  const allHave = lines.every(line => line.startsWith(prefix))
  const next = lines.map((line, index) => {
    if (allHave) return line.slice(prefix.length)
    const p = prefix === '1. ' ? `${index + 1}. ` : prefix
    return p + strip(line).replace(LIST, '$1')
  }).join('\n')
  return { text: next, start: lineStart, end: lineEnd, selStart: lineStart, selEnd: lineStart + next.length }
}

/** The edit Enter makes inside a list item: the next marker, or ending the list on an empty item. */
export function listContinuation(value: string, caret: number): Edit | null {
  const lineStart = value.lastIndexOf('\n', caret - 1) + 1
  const line = value.slice(lineStart, caret)
  const match = LIST.exec(line)
  if (!match) return null
  if (line.length === match[0].length) {
    return { text: '', start: lineStart, end: caret, selStart: lineStart, selEnd: lineStart }
  }
  const ordered = ORDERED.exec(line)
  const marker = ordered ? `${Number(ordered[2]) + 1}.` : match[2]
  const insert = '\n' + match[1] + marker + ' ' + (match[3] ? '[ ] ' : '')
  return { text: insert, start: caret, end: caret, selStart: caret + insert.length, selEnd: caret + insert.length }
}

/** The text typed after an unclosed `[[` on the caret's line, or null outside a wiki link. */
export function wikiLinkQuery(value: string, caret: number): { from: number; query: string } | null {
  const lineStart = value.lastIndexOf('\n', caret - 1) + 1
  const before = value.slice(lineStart, caret)
  const open = before.lastIndexOf('[[')
  if (open < 0 || before.indexOf(']]', open) >= 0) return null
  const query = before.slice(open + 2)
  if (/[\[\]|#]/.test(query) || query.length > 80) return null
  return { from: lineStart + open, query }
}

export function textStats(value: string) {
  const words = (value.match(/[\p{L}\p{N}][\p{L}\p{N}'’_-]*/gu) ?? []).length
  return { words, chars: value.length, lines: value ? value.split('\n').length : 1,
    minutes: Math.max(1, Math.round(words / 220)) }
}
</script>

<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import {
  Bold, Italic, Strikethrough, Code, Heading1, Heading2, Heading3, List, ListOrdered, ListChecks,
  Quote, SquareCode, Link, Brackets, Table, Minus, PenLine, Columns2, Eye, FileText, X, Maximize2, Minimize2,
} from '@lucide/vue'
import { renderMarkdown } from './lib/markdown'
import { useI18n } from './integration'
import { useTheme, useKnowledgeChartFetcher } from './integration'
import { renderMermaidBlocks } from './mermaidRenderer'
import { renderTradingChartBlocks } from './tradingChartRenderer'

const props = defineProps<{
  modelValue: string
  title: string
  folder: string
  fileName: string
  isDraft: boolean
  busy: boolean
  dirty: boolean
  saveDisabled: boolean
  saveLabel: string
  footnote?: string
  /** Wiki note paths (without `wiki/` and `.md`) offered after typing `[[`. */
  linkTargets: { path: string; title: string }[]
}>()
const emit = defineEmits<{
  'update:modelValue': [value: string]
  save: []
  close: []
  'markdown-click': [event: MouseEvent]
}>()

const { t } = useI18n()
const { dark } = useTheme()
const chartFetcher = useKnowledgeChartFetcher()
const MODE_KEY = 'knowledge.editor.mode'
function loadMode(): EditorMode {
  try {
    const saved = window.localStorage.getItem(MODE_KEY)
    if (saved === 'write' || saved === 'split' || saved === 'preview') return saved
  } catch { /* storage unavailable */ }
  return 'write'
}
const mode = ref<EditorMode>(loadMode())
const focusMode = ref(false)
watch(mode, value => { try { window.localStorage.setItem(MODE_KEY, value) } catch { /* ignore */ } })

const textarea = ref<HTMLTextAreaElement | null>(null)
const writePane = ref<HTMLElement | null>(null)
const previewPane = ref<HTMLElement | null>(null)
const caret = ref({ line: 1, col: 1 })
const stats = computed(() => textStats(props.modelValue))
const html = computed(() => mode.value === 'write' ? '' : renderMarkdown(props.modelValue))

const isMac = typeof navigator !== 'undefined' && /Mac|iP(hone|ad)/.test(navigator.platform)
const mod = isMac ? '⌘' : 'Ctrl+'

type Tool = { id: string; icon: unknown; label: string; keys?: string; run: () => void }
const tools = computed<Tool[][]>(() => [
  [
    { id: 'h1', icon: Heading1, label: t('knowledgeEditor.heading1'), run: () => prefix('# ') },
    { id: 'h2', icon: Heading2, label: t('knowledgeEditor.heading2'), run: () => prefix('## ') },
    { id: 'h3', icon: Heading3, label: t('knowledgeEditor.heading3'), run: () => prefix('### ') },
  ],
  [
    { id: 'bold', icon: Bold, label: t('knowledgeEditor.bold'), keys: mod + 'B', run: () => wrap('**', '**', t('knowledgeEditor.boldText')) },
    { id: 'italic', icon: Italic, label: t('knowledgeEditor.italic'), keys: mod + 'I', run: () => wrap('_', '_', t('knowledgeEditor.italicText')) },
    { id: 'strike', icon: Strikethrough, label: t('knowledgeEditor.strike'), run: () => wrap('~~', '~~') },
    { id: 'code', icon: Code, label: t('knowledgeEditor.inlineCode'), keys: mod + 'E', run: () => wrap('`', '`', 'code') },
  ],
  [
    { id: 'ul', icon: List, label: t('knowledgeEditor.bulletList'), run: () => prefix('- ') },
    { id: 'ol', icon: ListOrdered, label: t('knowledgeEditor.numberedList'), run: () => prefix('1. ') },
    { id: 'task', icon: ListChecks, label: t('knowledgeEditor.taskList'), run: () => prefix('- [ ] ') },
    { id: 'quote', icon: Quote, label: t('knowledgeEditor.quote'), run: () => prefix('> ') },
  ],
  [
    { id: 'link', icon: Link, label: t('knowledgeEditor.link'), keys: mod + 'K', run: insertLink },
    { id: 'wiki', icon: Brackets, label: t('knowledgeEditor.wikiLink'), run: () => wrap('[[', ']]') },
    { id: 'block', icon: SquareCode, label: t('knowledgeEditor.codeBlock'), run: () => block('```\n', '\n```', '') },
    { id: 'table', icon: Table, label: t('knowledgeEditor.table'), run: insertTable },
    { id: 'hr', icon: Minus, label: t('knowledgeEditor.divider'), run: () => block('', '\n---\n', '') },
  ],
])

function apply(edit: Edit) {
  const el = textarea.value
  if (!el) return
  el.focus()
  el.setSelectionRange(edit.start, edit.end)
  // insertText keeps the browser's own undo history; jsdom and old engines fall back to a plain write.
  let done = false
  try { done = typeof document.execCommand === 'function' && document.execCommand('insertText', false, edit.text) } catch { done = false }
  if (!done || el.value !== props.modelValue.slice(0, edit.start) + edit.text + props.modelValue.slice(edit.end)) {
    const next = props.modelValue.slice(0, edit.start) + edit.text + props.modelValue.slice(edit.end)
    el.value = next
    emit('update:modelValue', next)
  }
  void nextTick(() => { el.setSelectionRange(edit.selStart, edit.selEnd); autoGrow(); updateCaret() })
}
function selection() {
  const el = textarea.value!
  return { start: el.selectionStart, end: el.selectionEnd }
}
function ensureWritable(): boolean {
  if (props.busy || !textarea.value) return false
  if (mode.value === 'preview') mode.value = 'split'
  return true
}
function wrap(before: string, after: string, placeholder = '') {
  if (!ensureWritable()) return
  const { start, end } = selection()
  apply(wrapEdit(props.modelValue, start, end, before, after, placeholder))
}
function prefix(p: string) {
  if (!ensureWritable()) return
  const { start, end } = selection()
  apply(linePrefixEdit(props.modelValue, start, end, p))
}
function block(before: string, after: string, placeholder: string) {
  if (!ensureWritable()) return
  const { start, end } = selection()
  const lead = start > 0 && props.modelValue[start - 1] !== '\n' ? '\n' : ''
  apply(wrapEdit(props.modelValue, start, end, lead + before, after, placeholder))
}
function insertLink() {
  if (!ensureWritable()) return
  const { start, end } = selection()
  const label = props.modelValue.slice(start, end) || t('knowledgeEditor.linkText')
  const text = `[${label}](url)`
  const urlAt = start + label.length + 3
  apply({ text, start, end, selStart: urlAt, selEnd: urlAt + 3 })
}
function insertTable() {
  if (!ensureWritable()) return
  const { start, end } = selection()
  const lead = start > 0 && props.modelValue[start - 1] !== '\n' ? '\n\n' : ''
  const text = lead + '| Column | Column |\n| --- | --- |\n| Cell | Cell |\n'
  apply({ text, start, end, selStart: start + lead.length + 2, selEnd: start + lead.length + 8 })
}

// Wiki link suggestions after `[[`.
const suggest = ref<{ from: number; query: string } | null>(null)
const suggestIndex = ref(0)
const suggestPos = ref({ top: 0, left: 0 })
const suggestions = computed(() => {
  if (!suggest.value) return []
  const q = suggest.value.query.toLowerCase()
  return props.linkTargets
    .filter(item => !q || item.path.toLowerCase().includes(q) || item.title.toLowerCase().includes(q))
    .slice(0, 8)
})
function refreshSuggest() {
  const el = textarea.value
  if (!el || el.selectionStart !== el.selectionEnd) { suggest.value = null; return }
  const found = wikiLinkQuery(props.modelValue, el.selectionStart)
  if (found?.query !== suggest.value?.query) suggestIndex.value = 0
  suggest.value = found
  if (found) suggestPos.value = caretPoint(el, found.from)
}
function pickSuggestion(item: { path: string }) {
  const el = textarea.value
  if (!el || !suggest.value) return
  const end = el.selectionStart
  const closed = props.modelValue.slice(end, end + 2) === ']]'
  const text = '[[' + item.path + ']]'
  const from = suggest.value.from
  suggest.value = null
  apply({ text, start: from, end: end + (closed ? 2 : 0), selStart: from + text.length, selEnd: from + text.length })
}
/** Pixel position of `index` inside the textarea, measured with an off-screen mirror. */
function caretPoint(el: HTMLTextAreaElement, index: number) {
  const mirror = document.createElement('div')
  const style = window.getComputedStyle(el)
  for (const prop of ['fontFamily', 'fontSize', 'fontWeight', 'lineHeight', 'letterSpacing', 'paddingTop',
    'paddingLeft', 'paddingRight', 'borderTopWidth', 'borderLeftWidth', 'tabSize', 'wordSpacing'] as const) {
    mirror.style[prop] = style[prop]
  }
  Object.assign(mirror.style, { position: 'absolute', visibility: 'hidden', whiteSpace: 'pre-wrap',
    overflowWrap: 'break-word', width: el.clientWidth + 'px', top: '0', left: '-9999px' })
  mirror.textContent = el.value.slice(0, index)
  const marker = document.createElement('span')
  marker.textContent = '​'
  mirror.appendChild(marker)
  document.body.appendChild(mirror)
  const point = { top: el.offsetTop + marker.offsetTop + (parseFloat(style.lineHeight) || 24) - el.scrollTop,
    left: Math.min(el.offsetLeft + marker.offsetLeft, el.offsetLeft + el.clientWidth - 260) }
  mirror.remove()
  return point
}

function onInput(event: Event) {
  emit('update:modelValue', (event.target as HTMLTextAreaElement).value)
  void nextTick(() => { autoGrow(); refreshSuggest(); updateCaret() })
}
function onKeydown(event: KeyboardEvent) {
  const el = textarea.value
  if (!el) return
  if (suggest.value && suggestions.value.length) {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      const n = suggestions.value.length
      suggestIndex.value = (suggestIndex.value + (event.key === 'ArrowDown' ? 1 : n - 1)) % n
      return
    }
    if (event.key === 'Enter' || event.key === 'Tab') {
      event.preventDefault()
      pickSuggestion(suggestions.value[suggestIndex.value]!)
      return
    }
    if (event.key === 'Escape') { event.preventDefault(); suggest.value = null; return }
  }
  const meta = event.metaKey || event.ctrlKey
  if (meta && !event.altKey) {
    const key = event.key.toLowerCase()
    if (key === 's') { event.preventDefault(); emit('save'); return }
    if (key === 'b') { event.preventDefault(); wrap('**', '**', t('knowledgeEditor.boldText')); return }
    if (key === 'i') { event.preventDefault(); wrap('_', '_', t('knowledgeEditor.italicText')); return }
    if (key === 'e') { event.preventDefault(); wrap('`', '`', 'code'); return }
    if (key === 'k') { event.preventDefault(); insertLink(); return }
    if (key === '\\') { event.preventDefault(); cycleMode(); return }
  }
  if (event.key === 'Enter' && !event.shiftKey && !meta && !event.isComposing && el.selectionStart === el.selectionEnd) {
    const edit = listContinuation(props.modelValue, el.selectionStart)
    if (edit) { event.preventDefault(); apply(edit) }
    return
  }
  if (event.key === 'Tab' && !meta) {
    const { start, end } = selection()
    const multi = props.modelValue.slice(start, end).includes('\n')
    const lineStart = props.modelValue.lastIndexOf('\n', start - 1) + 1
    const inList = LIST.test(props.modelValue.slice(lineStart))
    if (!multi && !inList && !event.shiftKey) { event.preventDefault(); apply({ text: '  ', start, end, selStart: start + 2, selEnd: start + 2 }); return }
    event.preventDefault()
    const nextBreak = props.modelValue.indexOf('\n', end)
    const lineEnd = nextBreak < 0 ? props.modelValue.length : nextBreak
    const lines = props.modelValue.slice(lineStart, lineEnd).split('\n')
    const next = lines.map(line => event.shiftKey ? line.replace(/^ {1,2}/, '') : '  ' + line).join('\n')
    const shift = next.length - (lineEnd - lineStart)
    apply({ text: next, start: lineStart, end: lineEnd,
      selStart: multi ? lineStart : Math.max(lineStart, start + (event.shiftKey ? shift : 2)),
      selEnd: multi ? lineStart + next.length : Math.max(lineStart, end + (event.shiftKey ? shift : 2)) })
  }
  if (event.key === 'Escape' && focusMode.value) { event.preventDefault(); focusMode.value = false }
}
function onPaste(event: ClipboardEvent) {
  const el = textarea.value
  const pasted = event.clipboardData?.getData('text/plain')?.trim() ?? ''
  if (!el || el.selectionStart === el.selectionEnd || !/^https?:\/\/\S+$/.test(pasted)) return
  // Pasting a URL over selected text turns that text into a link.
  event.preventDefault()
  const { start, end } = selection()
  const text = `[${props.modelValue.slice(start, end)}](${pasted})`
  apply({ text, start, end, selStart: start + text.length, selEnd: start + text.length })
}
function cycleMode() {
  mode.value = mode.value === 'write' ? 'split' : mode.value === 'split' ? 'preview' : 'write'
}
function updateCaret() {
  const el = textarea.value
  if (!el) return
  const before = props.modelValue.slice(0, el.selectionStart)
  const line = before.split('\n').length
  caret.value = { line, col: el.selectionStart - (before.lastIndexOf('\n') + 1) + 1 }
}
function autoGrow() {
  const el = textarea.value
  if (!el) return
  el.style.height = 'auto'
  el.style.height = Math.max(el.scrollHeight, 360) + 'px'
}

// Split view keeps both panes at the same relative position.
let syncing: HTMLElement | null = null
function syncScroll(from: 'write' | 'preview') {
  if (mode.value !== 'split') return
  const source = from === 'write' ? writePane.value : previewPane.value
  const target = from === 'write' ? previewPane.value : writePane.value
  if (!source || !target || syncing === target) { syncing = null; return }
  const ratio = source.scrollTop / Math.max(1, source.scrollHeight - source.clientHeight)
  syncing = source
  target.scrollTop = ratio * (target.scrollHeight - target.clientHeight)
}

watch([html, dark, mode], () => {
  if (mode.value === 'write') return
  void nextTick(() => {
    void renderMermaidBlocks(previewPane.value, dark.value)
    if (chartFetcher) void renderTradingChartBlocks(previewPane.value, dark.value, { fetcher: chartFetcher })
  })
}, { flush: 'post' })
watch(mode, value => {
  if (value !== 'preview') void nextTick(() => { autoGrow(); textarea.value?.focus() })
})
watch(() => props.modelValue, () => {
  if (textarea.value && textarea.value.value !== props.modelValue) void nextTick(autoGrow)
})
onMounted(() => { void nextTick(autoGrow) })

const modes = computed(() => [
  { id: 'write' as const, icon: PenLine, label: t('knowledgeEditor.modeWrite') },
  { id: 'split' as const, icon: Columns2, label: t('knowledgeEditor.modeSplit') },
  { id: 'preview' as const, icon: Eye, label: t('knowledgeEditor.modePreview') },
])
</script>

<template>
  <form class="knowledge-note-editor kme" :class="['kme-mode-' + mode, { 'kme-focus': focusMode }]" @submit.prevent="emit('save')">
    <header class="knowledge-edit-toolbar kme-header">
      <div class="knowledge-edit-breadcrumb">
        <FileText :size="15" class="shrink-0" />
        <span>{{ folder }}/</span>
        <strong>{{ fileName }}</strong>
        <span v-if="isDraft" class="knowledge-draft-pill">{{ t('knowledgeV3.draftStatus') }}</span>
      </div>
      <div class="knowledge-edit-actions">
        <span class="kme-save-state" :class="{ 'is-dirty': dirty }" role="status">
          <span class="kme-dot" aria-hidden="true"></span>
          {{ dirty ? t('knowledgeV3.unsaved') : t('knowledgeV3.savedStatus') }}
        </span>
        <div class="kme-modes" role="radiogroup" :aria-label="t('knowledgeEditor.viewMode')">
          <button v-for="m in modes" :key="m.id" type="button" role="radio" :aria-checked="mode === m.id"
            :title="m.label + ' (' + mod + '\\)'" :aria-label="m.label" @click="mode = m.id">
            <component :is="m.icon" :size="14" /><span class="kme-mode-label">{{ m.label }}</span>
          </button>
        </div>
        <button type="button" class="kme-icon" :aria-pressed="focusMode"
          :title="t('knowledgeEditor.focusMode')" :aria-label="t('knowledgeEditor.focusMode')"
          @click="focusMode = !focusMode">
          <Minimize2 v-if="focusMode" :size="15" /><Maximize2 v-else :size="15" />
        </button>
        <button type="submit" class="knowledge-save-button" :disabled="saveDisabled" :title="saveLabel + ' (' + mod + 'S)'">
          {{ saveLabel }}
        </button>
        <button type="button" class="knowledge-close-editor" :aria-label="t('knowledgeV3.closeEditor')"
          :title="t('knowledgeV3.closeEditor')" @click="emit('close')"><X :size="16" /></button>
      </div>
    </header>

    <div v-if="mode !== 'preview'" class="kme-format" role="toolbar" :aria-label="t('knowledgeEditor.formatting')">
      <template v-for="(group, gi) in tools" :key="gi">
        <span v-if="gi" class="kme-sep" aria-hidden="true"></span>
        <button v-for="tool in group" :key="tool.id" type="button" class="kme-tool" :disabled="busy"
          :title="tool.label + (tool.keys ? ' (' + tool.keys + ')' : '')" :aria-label="tool.label"
          @mousedown.prevent @click="tool.run()">
          <component :is="tool.icon" :size="15" />
        </button>
      </template>
    </div>

    <div class="knowledge-edit-content kme-body">
      <div v-show="mode !== 'preview'" ref="writePane" class="knowledge-edit-note kme-write" @scroll.passive="syncScroll('write')">
        <h1>{{ title }}</h1>
        <div class="kme-textarea-wrap">
          <textarea id="knowledge-note-body" ref="textarea" :value="modelValue" :disabled="busy"
            :placeholder="t('knowledgeV3.startWriting')" spellcheck="true" aria-label="Markdown editor"
            autocomplete="off" @input="onInput" @keydown="onKeydown" @paste="onPaste"
            @click="updateCaret(); refreshSuggest()" @keyup="updateCaret" @blur="suggest = null"></textarea>
          <ul v-if="suggest && suggestions.length" class="kme-suggest" role="listbox"
            :aria-label="t('knowledgeEditor.wikiLink')"
            :style="{ top: suggestPos.top + 'px', left: Math.max(0, suggestPos.left) + 'px' }">
            <li v-for="(item, index) in suggestions" :key="item.path" role="option" :aria-selected="index === suggestIndex"
              :class="{ active: index === suggestIndex }" @mousedown.prevent="pickSuggestion(item)"
              @mouseenter="suggestIndex = index">
              <strong>{{ item.title }}</strong><span>{{ item.path }}</span>
            </li>
          </ul>
        </div>
      </div>
      <div v-if="mode !== 'write'" ref="previewPane" class="knowledge-edit-preview kme-preview" @scroll.passive="syncScroll('preview')">
        <h1 v-if="mode === 'preview'" class="kme-preview-title">{{ title }}</h1>
        <div v-if="modelValue.trim()" class="markdown-body" v-html="html" @click="emit('markdown-click', $event)"></div>
        <p v-else class="kme-empty">{{ t('knowledgeEditor.nothingToPreview') }}</p>
      </div>
    </div>

    <footer class="kme-status">
      <span>{{ t('knowledgeEditor.words', { count: stats.words }) }}</span>
      <span>{{ t('knowledgeEditor.chars', { count: stats.chars }) }}</span>
      <span>{{ t('knowledgeEditor.readTime', { count: stats.minutes }) }}</span>
      <span v-if="mode !== 'preview'">{{ t('knowledgeEditor.position', { line: caret.line, col: caret.col }) }}</span>
      <span class="kme-status-hint">{{ footnote }}</span>
      <span class="kme-status-md">Markdown</span>
    </footer>
  </form>
</template>

<script lang="ts">
import { stripMarkdownFrontmatter } from '@/lib/markdown'

/** Splits a note into its lead paragraph and the body that the reader and outline share. */
export function noteIntroduction(body: string, title: string): { lead: string; rest: string } {
  const markdown = stripMarkdownFrontmatter(body).trimStart()
  const first = /^#\s+(.+?)\s*(?:\r?\n|$)/.exec(markdown)
  const text = first && first[1]!.trim().toLowerCase() === title.trim().toLowerCase()
    ? markdown.slice(first[0].length).trimStart()
    : markdown
  const boundary = text.indexOf('\n\n')
  if (boundary < 0) return { lead: '', rest: text }
  const lead = text.slice(0, boundary).trim()
  if (!lead || /^(#{1,6}\s|[-*+]\s|>\s|\x60{3}|~{3}|\d+\.\s|\|)/.test(lead)) {
    return { lead: '', rest: text }
  }
  return { lead, rest: text.slice(boundary + 2).trimStart() }
}
</script>

<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { ChevronDown, ChevronRight } from '@lucide/vue'
import { parseMarkdownProperties, renderMarkdown } from '@/lib/markdown'
import { useI18n } from '@/composables/useI18n'
import { useTheme } from '@/composables/useTheme'
import { renderMermaidBlocks } from './mermaidRenderer'
import { renderTradingChartBlocks } from './tradingChartRenderer'
import type { NoteDoc, WorkspaceTab } from './useKnowledgeWorkspace'
import { useOptionalAdminKnowledgeBackend } from './knowledgeBackend'

const props = defineProps<{
  tab: WorkspaceTab
  doc: NoteDoc | null
  loading: boolean
  propertiesCollapsed: boolean
}>()
const emit = defineEmits<{
  'toggle-properties': []
  'markdown-click': [event: MouseEvent]
}>()

const { t } = useI18n()
const { dark } = useTheme()
const adminBackend = useOptionalAdminKnowledgeBackend()
const viewport = ref<HTMLElement | null>(null)

const properties = computed(() => parseMarkdownProperties(props.doc?.body ?? ''))
const introduction = computed(() => noteIntroduction(props.doc?.body ?? '', props.doc?.title ?? ''))
const leadHtml = computed(() => introduction.value.lead ? renderMarkdown(introduction.value.lead) : '')
const renderedBody = computed(() => renderMarkdown(introduction.value.rest))
const propertiesListId = computed(() => 'knowledge-doc-properties-' + props.tab.id)

function renderBlocks() {
  void renderMermaidBlocks(viewport.value, dark.value)
  if (adminBackend) void renderTradingChartBlocks(viewport.value, dark.value, { fetcher: fileName => adminBackend.chart(fileName) })
}
function onScroll() {
  if (viewport.value) props.tab.scrollTop = viewport.value.scrollTop
}
watch([renderedBody, () => props.doc?.body, dark, () => props.loading], renderBlocks, { flush: 'post' })
watch(() => props.doc?.path, (path, previous) => {
  // The same tab can navigate to another note; that note starts at its top.
  if (previous && path !== previous && viewport.value) {
    props.tab.scrollTop = 0
    viewport.value.scrollTop = 0
  }
})
onMounted(() => {
  renderBlocks()
  void nextTick(() => {
    if (viewport.value?.scrollTo) viewport.value.scrollTo({ top: props.tab.scrollTop, behavior: 'instant' })
  })
})
</script>

<template>
  <main ref="viewport" class="a-reader-viewport" @scroll.passive="onScroll">
    <article
      v-if="loading"
      data-knowledge-detail-skeleton
      role="status"
      aria-busy="true"
      :aria-label="t('common.loading')"
      class="doc"
    >
      <div class="card-shimmer space-y-5 rounded-xl p-1" aria-hidden="true">
        <div class="h-3 w-40 rounded bg-muted/50" />
        <div class="h-10 w-2/3 rounded-lg bg-muted/60" />
        <div class="space-y-2 pt-1">
          <div class="h-4 w-full rounded bg-muted/40" />
          <div class="h-4 w-5/6 rounded bg-muted/40" />
        </div>
        <div class="doc-divider"></div>
        <div class="space-y-3">
          <div class="h-6 w-48 rounded bg-muted/60" />
          <div class="h-3.5 w-full rounded bg-muted/40" />
          <div class="h-3.5 w-11/12 rounded bg-muted/40" />
          <div class="h-3.5 w-3/4 rounded bg-muted/40" />
        </div>
        <div class="space-y-3 pt-4">
          <div class="h-5 w-40 rounded bg-muted/60" />
          <div class="h-3.5 w-full rounded bg-muted/40" />
          <div class="h-3.5 w-4/5 rounded bg-muted/40" />
        </div>
      </div>
    </article>
    <article v-else-if="doc" class="doc">
      <div class="doc-overline"><span>{{ doc.space }} / Markdown note</span><span>·</span><span>Knowledge vault</span></div>
      <h1>{{ doc.title }}</h1>
      <section v-if="properties.length" class="doc-properties" aria-label="Note properties">
        <button type="button" class="doc-properties-toggle"
          :aria-expanded="!propertiesCollapsed"
          :aria-controls="propertiesListId"
          @click="emit('toggle-properties')">
          <ChevronRight v-if="propertiesCollapsed" :size="14" class="shrink-0 opacity-70" />
          <ChevronDown v-else :size="14" class="shrink-0 opacity-70" />
          <span>Properties</span>
          <span class="doc-properties-count">{{ properties.length }}</span>
        </button>
        <div v-if="!propertiesCollapsed" :id="propertiesListId" class="doc-properties-grid">
          <div v-for="prop in properties" :key="prop.key" class="doc-prop-row">
            <div class="doc-prop-key">{{ prop.label }}</div>
            <div class="doc-prop-val">
              <template v-if="prop.kind === 'badge'">
                <span v-for="val in prop.values" :key="val" class="doc-prop-badge">{{ val }}</span>
              </template>
              <template v-else-if="prop.kind === 'tags'">
                <span v-for="tag in prop.values" :key="tag" class="doc-prop-tag">#{{ tag }}</span>
              </template>
              <template v-else-if="prop.kind === 'code-list'">
                <code v-for="item in prop.values" :key="item" class="doc-prop-code">{{ item }}</code>
              </template>
              <template v-else-if="prop.kind === 'date'">
                <time class="doc-prop-date" :datetime="prop.raw || prop.values[0]" :title="prop.raw || prop.values[0]">{{ prop.values[0] }}</time>
              </template>
              <template v-else>
                <span class="doc-prop-text">{{ prop.values.join(', ') }}</span>
              </template>
            </div>
          </div>
        </div>
      </section>
      <div v-if="leadHtml" class="lead" v-html="leadHtml"></div>
      <div class="doc-divider"></div>
      <div class="markdown-body" v-html="renderedBody" @click="emit('markdown-click', $event)"
        @auxclick="emit('markdown-click', $event)"></div>
    </article>
  </main>
</template>

<script setup lang="ts">
import { computed, provide } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import KnowledgeWorkspacePage from '@vela/knowledge-ui/components/KnowledgePage.vue'
import {
  KNOWLEDGE_BACKEND_KEY,
  KNOWLEDGE_CHAT_PANEL_KEY,
  KNOWLEDGE_CHAT_PROVIDER_KEY,
  KNOWLEDGE_CHART_FETCHER_KEY,
  KNOWLEDGE_NAVIGATION_KEY,
} from '@vela/knowledge-ui'
import { useKnowledgeBackend } from './knowledgeBackend'
import { useWikiChat } from './useWikiChat'
import WikiChatPanel from './WikiChatPanel.vue'

const route = useRoute()
const router = useRouter()
const backend = useKnowledgeBackend()
provide(KNOWLEDGE_BACKEND_KEY, backend)
provide(KNOWLEDGE_NAVIGATION_KEY, {
  query: computed(() => route.query as Record<string, unknown>),
  replace: query => { void router.replace({ query: query as never }) },
})
provide(KNOWLEDGE_CHAT_PROVIDER_KEY, () => useWikiChat())
provide(KNOWLEDGE_CHAT_PANEL_KEY, WikiChatPanel)
const chartBackend = (backend as typeof backend & { admin?: { chart?: (fileName: string) => Promise<unknown> } }).admin
if (chartBackend?.chart) provide(KNOWLEDGE_CHART_FETCHER_KEY, fileName => chartBackend.chart!(fileName))
</script>

<template>
  <KnowledgeWorkspacePage />
</template>

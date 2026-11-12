import { createPinia } from 'pinia'
import { createApp } from 'vue'

import App from './App.vue'
import { router } from './router'
import './style.css'
import { KNOWLEDGE_BACKEND_KEY, HttpKnowledgeBackend } from './pages/knowledge/knowledgeBackend'
import { KNOWLEDGE_TRANSLATOR_KEY, KNOWLEDGE_DARK_MODE_KEY, KNOWLEDGE_CHART_FETCHER_KEY } from '@vela/knowledge-ui'
import { translate } from './composables/useI18n'
import { useTheme } from './composables/useTheme'

const knowledgeBackend = new HttpKnowledgeBackend()
const app = createApp(App)
app
  .provide(KNOWLEDGE_BACKEND_KEY, knowledgeBackend)
  .provide(KNOWLEDGE_TRANSLATOR_KEY, translate as unknown as (key: string, params?: Record<string, string | number>) => string)
  .provide(KNOWLEDGE_DARK_MODE_KEY, useTheme().dark)
  .provide(KNOWLEDGE_CHART_FETCHER_KEY, fileName => knowledgeBackend.admin.chart(fileName))
  .use(createPinia())
  .use(router)

// Resolve the initial auth redirect before mounting the shell and its API consumers.
void router.isReady().then(() => app.mount('#app'))

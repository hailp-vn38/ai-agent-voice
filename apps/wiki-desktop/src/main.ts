import { createApp } from 'vue'
import { ref } from 'vue'
import { KNOWLEDGE_BACKEND_KEY, KNOWLEDGE_DARK_MODE_KEY, KNOWLEDGE_NAVIGATION_KEY } from '@vela/knowledge-ui'
import App from './App.vue'
import { tauriKnowledgeBackend } from './tauriBackend'
import './style.css'

const dark = ref(matchMedia('(prefers-color-scheme: dark)').matches)
matchMedia('(prefers-color-scheme: dark)').addEventListener('change', event => { dark.value = event.matches })
const navQuery = ref(Object.fromEntries(new URLSearchParams(location.search)))

createApp(App)
  .provide(KNOWLEDGE_BACKEND_KEY, tauriKnowledgeBackend)
  .provide(KNOWLEDGE_DARK_MODE_KEY, dark)
  .provide(KNOWLEDGE_NAVIGATION_KEY, {
    query: navQuery,
    replace: (next: Record<string, unknown>) => {
      const params = new URLSearchParams()
      for (const [key, value] of Object.entries(next)) if (value != null) params.set(key, String(value))
      history.replaceState(null, '', `${location.pathname}?${params}`)
      navQuery.value = Object.fromEntries(params)
    },
  })
  .mount('#app')

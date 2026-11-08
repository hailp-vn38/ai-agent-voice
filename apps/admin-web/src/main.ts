import { createPinia } from 'pinia'
import { createApp } from 'vue'

import App from './App.vue'
import { router } from './router'
import './style.css'
import { KNOWLEDGE_BACKEND_KEY, HttpKnowledgeBackend } from './pages/knowledge/knowledgeBackend'

createApp(App)
  .provide(KNOWLEDGE_BACKEND_KEY, new HttpKnowledgeBackend())
  .use(createPinia())
  .use(router)
  .mount('#app')

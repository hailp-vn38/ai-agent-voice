<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import KnowledgePage from '@vela/knowledge-ui/components/KnowledgePage.vue'
import { VaultError } from '@vela/knowledge-ui'

const current = ref<string | null>(null)
const recent = ref<string[]>([])
const error = ref<VaultError | null>(null)
const loading = ref(false)
let windowNumber = 1

async function refreshVaults() {
  recent.value = await invoke<string[]>('vault_recent')
  current.value = await invoke<string | null>('vault_current')
}
async function openVault(path: string) {
  loading.value = true
  error.value = null
  try {
    current.value = await invoke<string>('vault_open', { path })
    await refreshVaults()
  } catch (cause) {
    const value = cause && typeof cause === 'object' ? cause as { kind?: VaultError['kind']; message?: string; current?: string } : {}
    error.value = new VaultError(value.kind ?? 'io', value.message ?? String(cause), value.current)
  } finally { loading.value = false }
}
async function pickVault() {
  loading.value = true
  error.value = null
  try {
    const picked = await invoke<string | null>('vault_pick')
    if (picked) { current.value = picked; await refreshVaults() }
  } catch (cause) {
    const value = cause && typeof cause === 'object' ? cause as { kind?: VaultError['kind']; message?: string; current?: string } : {}
    error.value = new VaultError(value.kind ?? 'io', value.message ?? String(cause), value.current)
  } finally { loading.value = false }
}
async function onShortcut(event: KeyboardEvent) {
  if (!event.metaKey && !event.ctrlKey) return
  const key = event.key.toLowerCase()
  if (key === 'o') { event.preventDefault(); await pickVault() }
  if (key === 'p' || (key === 'f' && event.shiftKey)) {
    event.preventDefault()
    document.querySelector<HTMLInputElement>('input[placeholder]')?.focus()
  }
  if (key === 'n' && event.shiftKey) {
    event.preventDefault()
    windowNumber += 1
    new WebviewWindow(`wiki-${windowNumber}`, { url: 'index.html', title: 'Wiki', width: 1280, height: 820 })
  }
  if (key === 'g') {
    event.preventDefault()
    const button = [...document.querySelectorAll('button')].find(item => /graph|đồ thị/i.test(`${item.getAttribute('aria-label')} ${item.title} ${item.textContent}`))
    button?.click()
  }
}

onMounted(async () => {
  try { await refreshVaults() } catch { /* WebView is launched by the bundled Tauri shell. */ }
  window.addEventListener('keydown', onShortcut)
})
onBeforeUnmount(() => window.removeEventListener('keydown', onShortcut))
</script>

<template>
  <main class="app-shell">
    <header class="titlebar" data-tauri-drag-region>
      <span class="app-mark" aria-hidden="true">W</span>
      <strong>Wiki</strong>
      <span v-if="current" class="vault-location" :title="current">{{ current.split('/').at(-1) }}</span>
      <button class="open-vault" type="button" @click="pickVault">Mở kho ⌘O</button>
    </header>
    <p v-if="error" class="error-banner" role="alert">
      <strong>{{ error.kind }}</strong><span>{{ error.message }}</span>
    </p>
    <section v-if="current" class="knowledge-frame" aria-label="Knowledge vault">
      <KnowledgePage />
    </section>
    <section v-else class="welcome" aria-labelledby="welcome-title">
      <div class="welcome-card">
        <span class="welcome-mark" aria-hidden="true">W</span>
        <h1 id="welcome-title">Mở kho tri thức</h1>
        <p>Chọn một thư mục Wiki trên máy Mac để duyệt nội dung.</p>
        <button class="primary-button" type="button" :disabled="loading" @click="pickVault">
          {{ loading ? 'Đang mở…' : 'Chọn thư mục' }}
        </button>
        <div v-if="recent.length" class="recent-list">
          <h2>Kho gần đây</h2>
          <button v-for="path in recent" :key="path" type="button" class="recent-item" @click="openVault(path)">
            <span>{{ path.split('/').at(-1) }}</span><small>{{ path }}</small>
          </button>
        </div>
      </div>
    </section>
  </main>
</template>

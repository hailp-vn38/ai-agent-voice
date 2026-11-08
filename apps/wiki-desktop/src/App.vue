<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import KnowledgePage from '@vela/knowledge-ui/components/KnowledgePage.vue'
import { VaultError, type VaultEvent } from '@vela/knowledge-ui'

type McpInfo = {
  port: number
  allowWrite: boolean
  url: string
  readConfig: string
  writeConfig: string
  stdioConfig: string
  status: { connectedClients: number; activeRequests: number; lastCallAt: number | null }
}
type ClaudePreview = { path: string; diff: { kind: 'same' | 'added' | 'removed'; text: string }[] }
type VaultSession = { current: string | null; recent: string[]; writable: boolean }

/** Tauri rejects with the serialized `CommandError` object, not an `Error`. */
function errorMessage(cause: unknown): string {
  if (cause && typeof cause === 'object' && 'message' in cause) return String((cause as { message: unknown }).message)
  return String(cause)
}

const current = ref<string | null>(null)
const recent = ref<string[]>([])
const error = ref<VaultError | null>(null)
const loading = ref(false)
const mcpOpen = ref(false)
const mcpInfo = ref<McpInfo | null>(null)
const mcpError = ref('')
const mcpToast = ref('')
const vaultWritable = ref(true)
const claudePreview = ref<ClaudePreview | null>(null)
const claudeInstalled = ref(false)
const requestedMcpPort = ref<number | null>(null)
let removeMcpStatus: (() => void) | undefined
let removeVaultEvents: (() => void) | undefined
let removeVaultLock: (() => void) | undefined
let toastTimer: ReturnType<typeof setTimeout> | undefined

async function refreshVaults() {
  const session = await invoke<VaultSession>('vault_session')
  recent.value = session.recent
  current.value = session.current
  vaultWritable.value = session.writable
}
async function openVault(path: string) {
  loading.value = true
  error.value = null
  try {
    current.value = await invoke<string>('vault_open', { path })
    vaultWritable.value = await invoke<boolean>('vault_is_writable')
    await refreshVaults()
    await refreshMcp()
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
    if (picked) { current.value = picked; await refreshVaults(); await refreshMcp() }
  } catch (cause) {
    const value = cause && typeof cause === 'object' ? cause as { kind?: VaultError['kind']; message?: string; current?: string } : {}
    error.value = new VaultError(value.kind ?? 'io', value.message ?? String(cause), value.current)
  } finally { loading.value = false }
}
async function refreshMcp() {
  try {
    mcpInfo.value = await invoke<McpInfo>('mcp_info')
    requestedMcpPort.value = mcpInfo.value.port
    mcpError.value = ''
  } catch (cause) {
    mcpInfo.value = null
    mcpError.value = errorMessage(cause)
  }
}
async function saveMcpPort() {
  if (!requestedMcpPort.value) return
  try {
    mcpInfo.value = await invoke<McpInfo>('mcp_set_port', { port: requestedMcpPort.value })
    requestedMcpPort.value = mcpInfo.value.port
    mcpError.value = ''
  } catch (cause) {
    mcpError.value = errorMessage(cause)
    await refreshMcp()
  }
}
async function toggleMcpWrite(event: Event) {
  const allowWrite = (event.target as HTMLInputElement).checked
  try {
    mcpInfo.value = await invoke<McpInfo>('mcp_set_allow_write', { allowWrite })
    mcpError.value = ''
  } catch (cause) {
    mcpError.value = errorMessage(cause)
    await refreshMcp()
  }
}
async function copyMcp(value: string) {
  try {
    await navigator.clipboard.writeText(value)
    mcpError.value = ''
  } catch (cause) {
    mcpError.value = `Không thể sao chép: ${String(cause)}`
  }
}
async function rotateMcpToken(access: 'read_only' | 'read_write') {
  try {
    mcpInfo.value = await invoke<McpInfo>('mcp_rotate_token', { access })
    mcpError.value = ''
  } catch (cause) {
    mcpError.value = errorMessage(cause)
  }
}
async function previewClaudeConfig() {
  try {
    claudePreview.value = await invoke<ClaudePreview>('mcp_claude_config_preview')
    claudeInstalled.value = false
    mcpError.value = ''
  } catch (cause) {
    mcpError.value = errorMessage(cause)
  }
}
async function installClaudeConfig() {
  if (!claudePreview.value) return
  try {
    await invoke('mcp_claude_config_install')
    claudeInstalled.value = true
    mcpError.value = ''
  } catch (cause) {
    mcpError.value = errorMessage(cause)
  }
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
    try { await invoke<string>('window_new') } catch (cause) { error.value = new VaultError('io', errorMessage(cause)) }
  }
  if (key === 'g') {
    event.preventDefault()
    const button = [...document.querySelectorAll('button')].find(item => /graph|đồ thị/i.test(`${item.getAttribute('aria-label')} ${item.title} ${item.textContent}`))
    button?.click()
  }
}

onMounted(async () => {
  try { await refreshVaults() } catch { /* WebView is launched by the bundled Tauri shell. */ }
  try {
    removeMcpStatus = await listen<McpInfo['status']>('mcp://status', event => {
      if (mcpInfo.value) mcpInfo.value.status = event.payload
    })
    removeVaultEvents = await listen<VaultEvent>('vault://changed', event => {
      if (event.payload.type !== 'changed' || event.payload.origin !== 'mcp') return
      const changedPath = event.payload.paths?.[0] ?? 'Wiki'
      mcpToast.value = `Agent đã cập nhật ${changedPath}`
      if (toastTimer) clearTimeout(toastTimer)
      toastTimer = setTimeout(() => { mcpToast.value = '' }, 3000)
    })
    removeVaultLock = await listen<{ writable: boolean }>('vault://lock', event => {
      vaultWritable.value = event.payload.writable
    })
  } catch { /* Native event API is available only inside the packaged Tauri shell. */ }
  if (current.value) await refreshMcp()
  window.addEventListener('keydown', onShortcut)
})
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onShortcut)
  removeMcpStatus?.()
  removeVaultEvents?.()
  removeVaultLock?.()
  if (toastTimer) clearTimeout(toastTimer)
})
</script>

<template>
  <main class="app-shell">
    <header class="titlebar" data-tauri-drag-region>
      <span class="app-mark" aria-hidden="true">W</span>
      <strong>Wiki</strong>
      <span v-if="current" class="vault-location" :title="current">{{ current.split('/').at(-1) }}</span>
      <button v-if="current" class="mcp-open" type="button" @click="mcpOpen = true; refreshMcp()">MCP</button>
      <button class="open-vault" type="button" @click="pickVault">Mở kho ⌘O</button>
    </header>
    <p v-if="error" class="error-banner" role="alert">
      <strong>{{ error.kind }}</strong><span>{{ error.message }}</span>
    </p>
    <p v-if="current && !vaultWritable" class="lock-banner" role="status">
      Kho đang được một tiến trình khác ghi. Wiki đang ở chế độ chỉ đọc và sẽ thử lấy lại quyền ghi.
    </p>
    <section v-if="current" class="knowledge-frame" aria-label="Knowledge vault">
      <KnowledgePage />
    </section>
    <footer v-if="current" class="mcp-footer" aria-label="MCP connection status">
      <button type="button" @click="mcpOpen = true; refreshMcp()">MCP · {{ mcpInfo?.status.connectedClients ?? 0 }} client đang kết nối</button>
      <span v-if="mcpInfo?.status.lastCallAt">Gọi gần nhất {{ new Date(mcpInfo.status.lastCallAt).toLocaleTimeString() }}</span>
    </footer>
    <div v-if="mcpToast" class="mcp-toast" role="status">{{ mcpToast }}</div>
    <div v-if="mcpOpen" class="mcp-overlay" role="presentation" @click.self="mcpOpen = false">
      <section class="mcp-settings" role="dialog" aria-modal="true" aria-labelledby="mcp-title">
        <header><div><p class="eyebrow">Kết nối agent</p><h1 id="mcp-title">MCP</h1></div>
          <button type="button" aria-label="Đóng MCP settings" @click="mcpOpen = false">×</button></header>
        <p v-if="mcpError" class="mcp-error" role="alert">{{ mcpError }}</p>
        <template v-if="mcpInfo">
          <label class="mcp-port-field">Port HTTP
            <input v-model.number="requestedMcpPort" type="number" min="1" max="65535" aria-label="MCP HTTP port">
            <button type="button" @click="saveMcpPort">Lưu port</button>
          </label>
          <p class="mcp-endpoint">HTTP MCP · <code>127.0.0.1:{{ mcpInfo.port }}/mcp</code></p>
          <label class="mcp-write-toggle"><input type="checkbox" :checked="mcpInfo.allowWrite" @change="toggleMcpWrite"> Cho phép agent ghi vào Wiki</label>
          <div class="mcp-token-row">
            <div><strong>Quyền đọc</strong><small>Token riêng, chỉ gọi được tool đọc.</small></div>
            <button type="button" @click="copyMcp(mcpInfo.readConfig)">Sao chép URL và token đọc</button>
            <button type="button" @click="rotateMcpToken('read_only')">Đổi token</button>
          </div>
          <div class="mcp-token-row">
            <div><strong>Quyền ghi</strong><small>Chỉ hoạt động khi bật quyền ghi.</small></div>
            <button type="button" :disabled="!mcpInfo.allowWrite" @click="copyMcp(mcpInfo.writeConfig)">Sao chép URL và token ghi</button>
            <button type="button" @click="rotateMcpToken('read_write')">Đổi token</button>
          </div>
          <div class="mcp-token-row">
            <div><strong>Claude Desktop · stdio</strong><small>Chuyển tiếp qua app khi đang chạy.</small></div>
            <button type="button" @click="copyMcp(mcpInfo.stdioConfig)">Sao chép cấu hình</button>
          </div>
          <button class="claude-install" type="button" @click="previewClaudeConfig">Cài vào Claude Desktop</button>
        </template>
        <p v-else class="mcp-empty">{{ mcpError || 'Đang kết nối MCP…' }}</p>
        <section v-if="claudePreview" class="claude-preview" aria-label="Claude Desktop config diff">
          <h2>{{ claudeInstalled ? 'Đã cập nhật cấu hình' : 'Xem thay đổi trước khi cài' }}</h2>
          <p>{{ claudePreview.path }}</p>
          <pre class="config-diff"><span v-for="(line, index) in claudePreview.diff" :key="index" :class="`diff-${line.kind}`">{{ line.kind === 'added' ? '+ ' : line.kind === 'removed' ? '- ' : '  ' }}{{ line.text }}</span></pre>
          <button v-if="!claudeInstalled" type="button" @click="installClaudeConfig">Xác nhận và ghi cấu hình</button>
        </section>
      </section>
    </div>
    <section v-if="!current" class="welcome" aria-labelledby="welcome-title">
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

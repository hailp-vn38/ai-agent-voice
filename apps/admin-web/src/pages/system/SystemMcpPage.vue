<script setup lang="ts">
import {
  ArrowLeft,
  BookOpen,
  Check,
  Copy,
  Eye,
  EyeOff,
  KeyRound,
  Link2,
  Plus,
  RefreshCcw,
  Server,
  Settings2,
  ShieldCheck,
  Terminal,
  Trash2,
  Wrench,
} from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { RouterLink } from 'vue-router'

import { formatApiError } from '@/api/errors'
import {
  systemMcpApi,
  type KnowledgeMcpAccess,
  type KnowledgeMcpCreatedToken,
  type KnowledgeMcpState,
  type KnowledgeMcpToken,
} from '@/api/systemMcp'
import PageHeader from '@/components/admin/PageHeader.vue'
import SystemMcpSettings from '@/components/system/SystemMcpSettings.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { useI18n } from '@/composables/useI18n'

const { locale, formatDateTime } = useI18n()
const isVi = computed(() => locale.value === 'vi')

const state = ref<KnowledgeMcpState | null>(null)
const loading = ref(true)
const savingConfig = ref(false)
const creatingToken = ref(false)
const deletingTokenId = ref('')
const error = ref('')
const notice = ref('')
let loadAbortController: AbortController | null = null

// Editable config form
const formEnabled = ref(true)
const formAllowWrite = ref(true)
const formMaxConcurrency = ref(16)
const formCallTimeoutMs = ref(30000)
const formMaxResultChars = ref(12000)

// Create token form
const newTokenName = ref('')
const newTokenAccess = ref<KnowledgeMcpAccess>('read_write')
const newTokenCustom = ref('')
const newlyCreatedToken = ref<KnowledgeMcpCreatedToken | null>(null)
const sessionPlaintextTokens = ref<Record<string, string>>({})

// UI helpers
const selectedTokenId = ref('')
const revealedTokens = ref<Record<string, boolean>>({})
const copiedKey = ref('')
const activeSnippetTab = ref<'mcp_json' | 'claude_cli' | 'headers' | 'curl'>('mcp_json')
const toolFilter = ref<'all' | 'read_only' | 'read_write'>('all')

function syncFormFromState(data: KnowledgeMcpState) {
  formEnabled.value = data.enabled
  formAllowWrite.value = data.allow_write
  formMaxConcurrency.value = data.max_concurrency
  formCallTimeoutMs.value = data.call_timeout_ms
  formMaxResultChars.value = data.max_result_chars
  for (const tok of data.tokens) {
    if (tok.token) {
      sessionPlaintextTokens.value[tok.id] = tok.token
    }
  }
  if (!selectedTokenId.value || !data.tokens.some((tok) => tok.id === selectedTokenId.value)) {
    selectedTokenId.value = data.tokens[0]?.id ?? ''
  }
}

const resolvedBaseUrl = computed(() => {
  const path = state.value?.path || '/mcp/knowledge'
  if (typeof window !== 'undefined' && window.location?.origin) {
    return `${window.location.origin}${path.startsWith('/') ? path : `/${path}`}`
  }
  return state.value?.base_url || `http://127.0.0.1:8000${path}`
})

function resolvePlaintextToken(tok: KnowledgeMcpToken): string | undefined {
  return sessionPlaintextTokens.value[tok.id] || tok.token
}

function displayTokenValue(tok: KnowledgeMcpToken): string {
  const plain = resolvePlaintextToken(tok)
  if (revealedTokens.value[tok.id] && plain) {
    return plain
  }
  return tok.masked_token || (plain ? maskToken(plain) : '••••••••')
}

const selectedTokenValue = computed(() => {
  const found = state.value?.tokens.find((tok) => tok.id === selectedTokenId.value)
  if (!found) return '<YOUR_MCP_TOKEN>'
  return resolvePlaintextToken(found) || '<YOUR_MCP_TOKEN>'
})

const snippetContent = computed(() => {
  const url = resolvedBaseUrl.value
  const token = selectedTokenValue.value
  if (activeSnippetTab.value === 'mcp_json') {
    return JSON.stringify(
      {
        mcpServers: {
          'voice-agent-knowledge': {
            type: 'streamable-http',
            url,
            headers: {
              Authorization: `Bearer ${token}`,
            },
          },
        },
      },
      null,
      2,
    )
  }
  if (activeSnippetTab.value === 'claude_cli') {
    return `claude mcp add --transport http voice-agent-knowledge "${url}" --header "Authorization: Bearer ${token}"`
  }
  if (activeSnippetTab.value === 'headers') {
    return `Endpoint URL: ${url}\nTransport: Streamable HTTP\nAuthorization: Bearer ${token}\nX-API-Key: ${token}`
  }
  return `curl -X POST "${url}" \\\n  -H "Authorization: Bearer ${token}" \\\n  -H "Content-Type: application/json" \\\n  -H "Accept: application/json, text/event-stream" \\\n  -H "MCP-Protocol-Version: 2025-11-25" \\\n  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'`
})

const filteredTools = computed(() => {
  const list = state.value?.tools ?? []
  if (toolFilter.value === 'all') return list
  return list.filter((tool) => tool.access === toolFilter.value)
})

const configDirty = computed(() => {
  if (!state.value) return false
  return (
    formEnabled.value !== state.value.enabled ||
    formAllowWrite.value !== state.value.allow_write ||
    Number(formMaxConcurrency.value) !== state.value.max_concurrency ||
    Number(formCallTimeoutMs.value) !== state.value.call_timeout_ms ||
    Number(formMaxResultChars.value) !== state.value.max_result_chars
  )
})

async function copyText(key: string, text: string) {
  try {
    await navigator.clipboard.writeText(text)
    copiedKey.value = key
    window.setTimeout(() => {
      if (copiedKey.value === key) copiedKey.value = ''
    }, 2000)
  } catch {
    error.value = isVi.value ? 'Không thể sao chép vào clipboard' : 'Clipboard unavailable'
  }
}

function toggleReveal(id: string) {
  revealedTokens.value[id] = !revealedTokens.value[id]
}

function maskToken(token: string) {
  if (token.length <= 12) return '••••••••'
  return `${token.slice(0, 8)}••••••••${token.slice(-4)}`
}

async function load() {
  loadAbortController?.abort()
  const controller = new AbortController()
  loadAbortController = controller
  loading.value = true
  error.value = ''
  try {
    const res = await systemMcpApi.knowledge(controller.signal)
    state.value = res
    syncFormFromState(res)
  } catch (cause) {
    if ((cause as { name?: string })?.name === 'AbortError') return
    error.value = formatApiError(cause)
  } finally {
    if (loadAbortController === controller) {
      loading.value = false
    }
  }
}

async function saveConfig() {
  savingConfig.value = true
  error.value = ''
  notice.value = ''
  try {
    const updated = await systemMcpApi.saveKnowledge(
      {
        enabled: formEnabled.value,
        allow_write: formAllowWrite.value,
        max_concurrency: Number(formMaxConcurrency.value),
        call_timeout_ms: Number(formCallTimeoutMs.value),
        max_result_chars: Number(formMaxResultChars.value),
      },
      state.value?.revision,
    )
    state.value = updated
    syncFormFromState(updated)
    notice.value = isVi.value
      ? 'Đã lưu cấu hình Knowledge MCP Server (áp dụng tức thì).'
      : 'Knowledge MCP configuration saved and applied immediately.'
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    savingConfig.value = false
  }
}

async function handleCreateToken() {
  const name = newTokenName.value.trim()
  if (!name) return
  creatingToken.value = true
  error.value = ''
  notice.value = ''
  try {
    const updated = await systemMcpApi.createToken({
      name,
      access: newTokenAccess.value,
      custom_token: newTokenCustom.value.trim() || undefined,
    })
    if (updated.created_token) {
      sessionPlaintextTokens.value[updated.created_token.id] = updated.created_token.token
      newlyCreatedToken.value = updated.created_token
      selectedTokenId.value = updated.created_token.id
      revealedTokens.value[updated.created_token.id] = true
    }
    state.value = updated
    syncFormFromState(updated)
    newTokenName.value = ''
    newTokenCustom.value = ''
    notice.value = isVi.value ? 'Đã tạo token MCP mới thành công.' : 'New MCP token created.'
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    creatingToken.value = false
  }
}

async function handleDeleteToken(tok: KnowledgeMcpToken) {
  deletingTokenId.value = tok.id
  error.value = ''
  notice.value = ''
  try {
    const updated = await systemMcpApi.deleteToken(tok.id)
    delete sessionPlaintextTokens.value[tok.id]
    state.value = updated
    if (newlyCreatedToken.value?.id === tok.id) {
      newlyCreatedToken.value = null
    }
    syncFormFromState(updated)
    notice.value = isVi.value
      ? `Đã thu hồi token "${tok.name}".`
      : `Revoked token "${tok.name}".`
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    deletingTokenId.value = ''
  }
}

onMounted(() => {
  void load()
})

onBeforeUnmount(() => {
  loadAbortController?.abort()
})
</script>

<template>
  <section class="space-y-6">
    <PageHeader
      :eyebrow="isVi ? 'HẠ TẦNG HỆ THỐNG' : 'SYSTEM INFRASTRUCTURE'"
      :title="isVi ? 'System & Knowledge MCP Server' : 'System & Knowledge MCP Server'"
      :description="
        isVi
          ? 'Quản lý cổng Streamable HTTP MCP (/mcp/knowledge), khởi tạo Bearer Token / API Key, lấy Base URL và cấu hình kết nối trực tiếp cho các Client Agent bên ngoài.'
          : 'Manage the Streamable HTTP Knowledge MCP endpoint (/mcp/knowledge), issue Bearer / API-Key tokens, copy Base URLs, and configure external Client Agents.'
      "
    >
      <template #actions>
        <RouterLink
          to="/system"
          class="inline-flex h-9 items-center justify-center gap-2 rounded-md border border-input bg-background px-3 text-sm font-medium text-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        >
          <ArrowLeft class="size-4" />
          <span>{{ isVi ? 'Quay lại Hệ thống' : 'Back to System' }}</span>
        </RouterLink>
        <Button variant="outline" :disabled="loading" @click="load">
          <RefreshCcw :class="['size-4', loading && 'animate-spin']" />
          <span>{{ isVi ? 'Làm mới' : 'Refresh' }}</span>
        </Button>
      </template>
    </PageHeader>

    <div
      v-if="error"
      role="alert"
      class="rounded-lg border border-destructive/40 bg-destructive/5 p-3 text-sm text-destructive"
    >
      {{ error }}
    </div>
    <div
      v-if="notice"
      role="status"
      class="rounded-lg border border-emerald-500/40 bg-emerald-500/5 p-3 text-sm text-emerald-700 dark:text-emerald-300"
    >
      {{ notice }}
    </div>

    <!-- Summary Strip -->
    <div class="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
      <Card>
        <CardContent class="flex items-center justify-between pt-6">
          <div class="space-y-1">
            <p class="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              {{ isVi ? 'Trạng thái MCP' : 'MCP Status' }}
            </p>
            <div class="flex items-center gap-2">
              <Badge :variant="state?.enabled ? 'success' : 'secondary'" data-testid="mcp-status-badge">
                {{ state?.enabled ? (isVi ? 'Đang bật (Online)' : 'Enabled') : isVi ? 'Đang tắt' : 'Disabled' }}
              </Badge>
              <Badge variant="outline">
                {{ state?.allow_write ? 'Read-Write' : 'Read-Only' }}
              </Badge>
            </div>
          </div>
          <Server class="size-6 text-studio-violet" />
        </CardContent>
      </Card>

      <Card>
        <CardContent class="flex items-center justify-between pt-6">
          <div class="min-w-0 flex-1 space-y-1 pr-2">
            <p class="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              {{ isVi ? 'Endpoint Path' : 'Endpoint Path' }}
            </p>
            <p class="truncate font-mono text-sm font-semibold">
              {{ state?.path ?? '/mcp/knowledge' }}
            </p>
          </div>
          <Link2 class="size-6 shrink-0 text-studio-cyan" />
        </CardContent>
      </Card>

      <Card>
        <CardContent class="flex items-center justify-between pt-6">
          <div class="space-y-1">
            <p class="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              {{ isVi ? 'Token đang hoạt động' : 'Active Tokens' }}
            </p>
            <p class="text-2xl font-bold">{{ state?.tokens.length ?? 0 }}</p>
          </div>
          <KeyRound class="size-6 text-amber-500" />
        </CardContent>
      </Card>

      <Card>
        <CardContent class="flex items-center justify-between pt-6">
          <div class="space-y-1">
            <p class="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              {{ isVi ? 'Công cụ MCP (Read / Full)' : 'MCP Tools (Read / Full)' }}
            </p>
            <p class="text-2xl font-bold">
              {{ state?.read_only_tools_count ?? 7 }} / {{ state?.read_write_tools_count ?? 16 }}
            </p>
          </div>
          <Wrench class="size-6 text-emerald-500" />
        </CardContent>
      </Card>
    </div>

    <!-- Base URL & Client Config Generator -->
    <Card>
      <CardHeader>
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div>
            <CardTitle class="flex items-center gap-2">
              <Terminal class="size-5 text-studio-violet" />
              <span>{{ isVi ? 'Base URL & Cấu hình kết nối Client Agent' : 'Base URL & Client Agent Connection Config' }}</span>
            </CardTitle>
            <CardDescription class="mt-1">
              {{
                isVi
                  ? 'Sao chép trực tiếp Base URL hoặc đoạn mã cấu hình chuẩn Streamable HTTP MCP cho Claude Code, Cursor, Claude Desktop hoặc Agent tùy chỉnh.'
                  : 'Copy the Streamable HTTP MCP Base URL or ready-to-use configuration snippets for Claude Code, Cursor, Claude Desktop, or custom Agents.'
              }}
            </CardDescription>
          </div>
        </div>
      </CardHeader>
      <CardContent class="space-y-4">
        <div class="grid gap-4 lg:grid-cols-3">
          <div class="space-y-1.5 lg:col-span-2">
            <label class="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              {{ isVi ? 'MCP Base URL (Streamable HTTP)' : 'MCP Base URL (Streamable HTTP)' }}
            </label>
            <div class="flex items-center gap-2">
              <input
                :value="resolvedBaseUrl"
                readonly
                data-testid="mcp-base-url-input"
                class="admin-input flex-1 font-mono text-sm"
              />
              <Button
                type="button"
                variant="outline"
                data-testid="copy-base-url-btn"
                @click="copyText('base_url', resolvedBaseUrl)"
              >
                <Check v-if="copiedKey === 'base_url'" class="size-4 text-emerald-500" />
                <Copy v-else class="size-4" />
                <span>{{ copiedKey === 'base_url' ? (isVi ? 'Đã chép' : 'Copied') : isVi ? 'Copy URL' : 'Copy URL' }}</span>
              </Button>
            </div>
          </div>

          <div class="space-y-1.5">
            <label class="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              {{ isVi ? 'Chọn Token điền vào Config' : 'Token for Config Snippet' }}
            </label>
            <select v-model="selectedTokenId" class="admin-input w-full text-sm">
              <option v-if="!state?.tokens.length" value="">
                {{ isVi ? '— Chưa có token (Hãy tạo bên dưới) —' : '— No token yet (Create one below) —' }}
              </option>
              <option v-for="tok in state?.tokens ?? []" :key="tok.id" :value="tok.id">
                {{ tok.name }} ({{ tok.access === 'read_write' ? 'Read-Write' : 'Read-Only' }})
              </option>
            </select>
          </div>
        </div>

        <!-- Snippet Tabs -->
        <div class="space-y-2">
          <div class="flex flex-wrap items-center justify-between gap-2 border-b border-border pb-2">
            <div class="flex flex-wrap gap-1.5">
              <Button
                type="button"
                size="sm"
                :variant="activeSnippetTab === 'mcp_json' ? 'default' : 'ghost'"
                @click="activeSnippetTab = 'mcp_json'"
              >
                MCP JSON (Cursor / Claude)
              </Button>
              <Button
                type="button"
                size="sm"
                :variant="activeSnippetTab === 'claude_cli' ? 'default' : 'ghost'"
                @click="activeSnippetTab = 'claude_cli'"
              >
                Claude Code CLI
              </Button>
              <Button
                type="button"
                size="sm"
                :variant="activeSnippetTab === 'headers' ? 'default' : 'ghost'"
                @click="activeSnippetTab = 'headers'"
              >
                HTTP Headers
              </Button>
              <Button
                type="button"
                size="sm"
                :variant="activeSnippetTab === 'curl' ? 'default' : 'ghost'"
                @click="activeSnippetTab = 'curl'"
              >
                cURL Test
              </Button>
            </div>
            <Button
              type="button"
              size="sm"
              variant="outline"
              data-testid="copy-snippet-btn"
              @click="copyText('snippet', snippetContent)"
            >
              <Check v-if="copiedKey === 'snippet'" class="size-4 text-emerald-500" />
              <Copy v-else class="size-4" />
              <span>{{ copiedKey === 'snippet' ? (isVi ? 'Đã chép Config' : 'Copied') : isVi ? 'Copy Config' : 'Copy Config' }}</span>
            </Button>
          </div>
          <pre
            data-testid="mcp-config-snippet"
            class="overflow-x-auto rounded-lg border border-border bg-muted/40 p-4 font-mono text-xs leading-relaxed"
          ><code>{{ snippetContent }}</code></pre>
        </div>
      </CardContent>
    </Card>

    <div class="grid gap-6 lg:grid-cols-2">
      <!-- Token Management Card -->
      <Card>
        <CardHeader>
          <CardTitle class="flex items-center gap-2">
            <KeyRound class="size-5 text-amber-500" />
            <span>{{ isVi ? 'Tạo & Quản lý Token truy cập' : 'Create & Manage Access Tokens' }}</span>
          </CardTitle>
          <CardDescription>
            {{
              isVi
                ? 'Tạo Bearer Token / X-API-Key riêng cho từng Client Agent với quyền Read-Only (chỉ tra cứu) hoặc Read-Write (đọc, ghi Wiki & stage vào sources/).'
                : 'Issue dedicated Bearer / X-API-Key tokens with Read-Only or Read-Write permissions.'
            }}
          </CardDescription>
        </CardHeader>
        <CardContent class="space-y-5">
          <form class="space-y-3 rounded-lg border border-border bg-muted/20 p-4" @submit.prevent="handleCreateToken">
            <div class="grid gap-3 sm:grid-cols-2">
              <label class="space-y-1 text-sm">
                <span class="font-medium">{{ isVi ? 'Tên Token / Agent' : 'Token / Agent Name' }}</span>
                <input
                  v-model="newTokenName"
                  type="text"
                  required
                  maxlength="128"
                  data-testid="new-token-name-input"
                  class="admin-input w-full"
                  :placeholder="isVi ? 'VD: Claude Code, Cursor Agent...' : 'e.g. Claude Code, Cursor Agent...'"
                />
              </label>
              <label class="space-y-1 text-sm">
                <span class="font-medium">{{ isVi ? 'Quyền hạn (Scope)' : 'Permission Scope' }}</span>
                <select
                  v-model="newTokenAccess"
                  data-testid="new-token-access-select"
                  class="admin-input w-full"
                >
                  <option value="read_write">
                    {{ isVi ? 'Read-Write (Đọc & Ghi Wiki / Stage Sources)' : 'Read-Write (Full Wiki & Stage Sources)' }}
                  </option>
                  <option value="read_only">
                    {{ isVi ? 'Read-Only (Chỉ đọc & Tra cứu Wiki)' : 'Read-Only (Query & Read Only)' }}
                  </option>
                </select>
              </label>
            </div>
            <label class="block space-y-1 text-sm">
              <span class="font-medium">
                {{ isVi ? 'Chuỗi Token tùy chọn (Để trống để tự sinh ngẫu nhiên)' : 'Custom Token (Leave blank to auto-generate)' }}
              </span>
              <input
                v-model="newTokenCustom"
                type="text"
                minlength="8"
                maxlength="512"
                class="admin-input w-full font-mono text-xs"
                :placeholder="isVi ? 'Tự động sinh kmcp_... nếu để trống' : 'Auto-generates kmcp_... when left empty'"
              />
            </label>
            <Button
              type="submit"
              :disabled="creatingToken || !newTokenName.trim()"
              data-testid="create-token-submit-btn"
            >
              <Plus class="size-4" />
              <span>{{ isVi ? 'Tạo Token mới' : 'Create Token' }}</span>
            </Button>
          </form>

          <!-- Newly Created Token Highlight -->
          <div
            v-if="newlyCreatedToken"
            data-testid="newly-created-token-banner"
            class="space-y-2 rounded-lg border border-emerald-500/40 bg-emerald-500/10 p-3 text-sm"
          >
            <div class="flex items-center justify-between gap-2">
              <span class="font-semibold text-emerald-700 dark:text-emerald-300">
                {{ isVi ? `Token mới tạo: ${newlyCreatedToken.name}` : `Newly created token: ${newlyCreatedToken.name}` }}
              </span>
              <Button
                type="button"
                size="sm"
                variant="outline"
                @click="copyText('new_tok', newlyCreatedToken.token)"
              >
                <Check v-if="copiedKey === 'new_tok'" class="size-3.5 text-emerald-500" />
                <Copy v-else class="size-3.5" />
                <span>{{ copiedKey === 'new_tok' ? (isVi ? 'Đã chép' : 'Copied') : 'Copy' }}</span>
              </Button>
            </div>
            <code class="block break-all rounded bg-background/80 p-2 font-mono text-xs">
              {{ newlyCreatedToken.token }}
            </code>
          </div>

          <!-- Token List -->
          <div class="space-y-2">
            <h3 class="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              {{ isVi ? 'Danh sách Token hiện có' : 'Existing Tokens' }} ({{ state?.tokens.length ?? 0 }})
            </h3>
            <p v-if="!state?.tokens.length" class="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">
              {{ isVi ? 'Chưa có token nào. Hãy tạo một token ở khung phía trên.' : 'No tokens configured yet. Create one above.' }}
            </p>
            <div v-else class="space-y-2.5">
              <div
                v-for="tok in state.tokens"
                :key="tok.id"
                data-testid="mcp-token-row"
                class="flex flex-col justify-between gap-3 rounded-lg border border-border p-3 text-sm sm:flex-row sm:items-center"
              >
                <div class="min-w-0 flex-1 space-y-1">
                  <div class="flex flex-wrap items-center gap-2">
                    <span class="font-semibold">{{ tok.name }}</span>
                    <Badge :variant="tok.access === 'read_write' ? 'default' : 'secondary'">
                      {{ tok.access === 'read_write' ? 'Read-Write' : 'Read-Only' }}
                    </Badge>
                  </div>
                  <p class="break-all font-mono text-xs text-muted-foreground">
                    {{ displayTokenValue(tok) }}
                  </p>
                  <p v-if="tok.created_at" class="text-[11px] text-muted-foreground">
                    {{ formatDateTime(new Date(tok.created_at * 1000)) }}
                  </p>
                </div>
                <div class="flex shrink-0 items-center gap-1.5">
                  <Button
                    v-if="resolvePlaintextToken(tok)"
                    type="button"
                    size="sm"
                    variant="ghost"
                    :title="revealedTokens[tok.id] ? 'Hide' : 'Show'"
                    @click="toggleReveal(tok.id)"
                  >
                    <EyeOff v-if="revealedTokens[tok.id]" class="size-4" />
                    <Eye v-else class="size-4" />
                  </Button>
                  <Button
                    v-if="resolvePlaintextToken(tok)"
                    type="button"
                    size="sm"
                    variant="outline"
                    @click="copyText(`tok_${tok.id}`, resolvePlaintextToken(tok)!)"
                  >
                    <Check v-if="copiedKey === `tok_${tok.id}`" class="size-3.5 text-emerald-500" />
                    <Copy v-else class="size-3.5" />
                    <span>{{ copiedKey === `tok_${tok.id}` ? (isVi ? 'Đã chép' : 'Copied') : 'Copy' }}</span>
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    class="text-destructive hover:text-destructive"
                    :disabled="deletingTokenId === tok.id"
                    data-testid="delete-token-btn"
                    @click="handleDeleteToken(tok)"
                  >
                    <Trash2 class="size-4" />
                  </Button>
                </div>
              </div>
            </div>
          </div>
        </CardContent>
      </Card>

      <!-- Web Server Configuration Card -->
      <Card>
        <CardHeader>
          <CardTitle class="flex items-center gap-2">
            <Settings2 class="size-5 text-studio-cyan" />
            <span>{{ isVi ? 'Cấu hình MCP Server trên Web' : 'Web MCP Server Configuration' }}</span>
          </CardTitle>
          <CardDescription>
            {{
              isVi
                ? 'Điều chỉnh trạng thái bật/tắt, quyền ghi và giới hạn tài nguyên của Knowledge MCP Server ngay trên giao diện Web (có hiệu lực tức thì).'
                : 'Configure enabled state, write permissions, and execution limits directly from the web UI (applies immediately without restart).'
            }}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form class="space-y-4" @submit.prevent="saveConfig">
            <label class="flex items-start gap-3 rounded-lg border border-border p-3 cursor-pointer">
              <input
                v-model="formEnabled"
                type="checkbox"
                data-testid="mcp-enabled-checkbox"
                class="mt-1"
              />
              <div class="space-y-0.5 text-sm">
                <p class="font-medium">
                  {{ isVi ? 'Bật Knowledge MCP Server (/mcp/knowledge)' : 'Enable Knowledge MCP Server (/mcp/knowledge)' }}
                </p>
                <p class="text-xs text-muted-foreground">
                  {{
                    isVi
                      ? 'Khi tắt, mọi yêu cầu tới /mcp/knowledge sẽ trả về 404 Not Found.'
                      : 'When disabled, requests to /mcp/knowledge return 404 Not Found.'
                  }}
                </p>
              </div>
            </label>

            <label class="flex items-start gap-3 rounded-lg border border-border p-3 cursor-pointer">
              <input
                v-model="formAllowWrite"
                type="checkbox"
                data-testid="mcp-allow-write-checkbox"
                class="mt-1"
              />
              <div class="space-y-0.5 text-sm">
                <p class="font-medium">
                  {{
                    isVi
                      ? 'Cho phép công cụ Ghi / Ingest / Stage Sources (allow_write)'
                      : 'Allow Write, Ingest & Source Staging Tools (allow_write)'
                  }}
                </p>
                <p class="text-xs text-muted-foreground">
                  {{
                    isVi
                      ? 'Khi tắt, tất cả các token (kể cả Read-Write) sẽ chỉ được phép gọi 7 công cụ tra cứu Read-Only.'
                      : 'When unchecked, all tokens are restricted to the 7 Read-Only query tools.'
                  }}
                </p>
              </div>
            </label>

            <div class="grid gap-4 sm:grid-cols-3">
              <label class="space-y-1 text-sm">
                <span class="font-medium">{{ isVi ? 'Max Concurrency' : 'Max Concurrency' }}</span>
                <input
                  v-model.number="formMaxConcurrency"
                  type="number"
                  min="1"
                  max="64"
                  required
                  class="admin-input w-full"
                />
              </label>

              <label class="space-y-1 text-sm">
                <span class="font-medium">{{ isVi ? 'Timeout (ms)' : 'Call Timeout (ms)' }}</span>
                <input
                  v-model.number="formCallTimeoutMs"
                  type="number"
                  min="1000"
                  max="120000"
                  step="1000"
                  required
                  class="admin-input w-full"
                />
              </label>

              <label class="space-y-1 text-sm">
                <span class="font-medium">{{ isVi ? 'Max Result Chars' : 'Max Result Chars' }}</span>
                <input
                  v-model.number="formMaxResultChars"
                  type="number"
                  min="512"
                  max="64000"
                  step="512"
                  required
                  class="admin-input w-full"
                />
              </label>
            </div>

            <div class="flex flex-wrap items-center justify-between gap-3 pt-2">
              <span class="text-xs text-muted-foreground">
                Revision: <code>{{ state?.revision ?? 1 }}</code>
              </span>
              <Button
                type="submit"
                :disabled="savingConfig || !configDirty"
                data-testid="save-mcp-config-btn"
              >
                <ShieldCheck class="size-4" />
                <span>{{ isVi ? 'Lưu cấu hình MCP' : 'Save MCP Configuration' }}</span>
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>
    </div>

    <!-- Exposed Tools Catalog -->
    <Card>
      <CardHeader>
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div>
            <CardTitle class="flex items-center gap-2">
              <BookOpen class="size-5 text-emerald-500" />
              <span>{{ isVi ? 'Danh sách Công cụ Knowledge MCP' : 'Exposed Knowledge MCP Tools' }}</span>
            </CardTitle>
            <CardDescription class="mt-1">
              {{
                isVi
                  ? 'Tất cả JSON Schema của công cụ đều đã được chuẩn hóa tự động để tương thích với ExternalMcpClient và các MCP Client tiêu chuẩn.'
                  : 'All tool JSON schemas are automatically sanitized for full compatibility with ExternalMcpClient and standard MCP clients.'
              }}
            </CardDescription>
          </div>
          <div class="flex gap-1.5">
            <Button
              type="button"
              size="sm"
              :variant="toolFilter === 'all' ? 'default' : 'outline'"
              @click="toolFilter = 'all'"
            >
              {{ isVi ? 'Tất cả' : 'All' }} ({{ state?.tools.length ?? 0 }})
            </Button>
            <Button
              type="button"
              size="sm"
              :variant="toolFilter === 'read_only' ? 'default' : 'outline'"
              @click="toolFilter = 'read_only'"
            >
              Read-Only ({{ state?.read_only_tools_count ?? 0 }})
            </Button>
            <Button
              type="button"
              size="sm"
              :variant="toolFilter === 'read_write' ? 'default' : 'outline'"
              @click="toolFilter = 'read_write'"
            >
              Write / Ingest ({{ (state?.read_write_tools_count ?? 0) - (state?.read_only_tools_count ?? 0) }})
            </Button>
          </div>
        </div>
      </CardHeader>
      <CardContent>
        <div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
          <div
            v-for="tool in filteredTools"
            :key="tool.name"
            class="flex flex-col justify-between gap-2 rounded-lg border border-border p-3.5 text-sm"
          >
            <div class="space-y-1.5">
              <div class="flex items-center justify-between gap-2">
                <code class="font-mono text-xs font-bold text-studio-violet">{{ tool.name }}</code>
                <Badge :variant="tool.access === 'read_only' ? 'secondary' : 'default'">
                  {{ tool.access === 'read_only' ? 'Read-Only' : 'Write' }}
                </Badge>
              </div>
              <p class="text-xs leading-relaxed text-muted-foreground">
                {{ tool.description }}
              </p>
            </div>
          </div>
        </div>
      </CardContent>
    </Card>

    <!-- Studio Assistant System Tool Grants & Change Requests -->
    <SystemMcpSettings />
  </section>
</template>

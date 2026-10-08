<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'

import type { AdminMcpServer, CreateMcpServerInput, McpAuthInput, UpdateMcpServerInput } from '@/api/types/mcp'
import BaseModal from '@/components/admin/BaseModal.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{ server?: AdminMcpServer; saving: boolean; serverError?: string }>()
const open = defineModel<boolean>({ required: true })
const emit = defineEmits<{ save: [payload: CreateMcpServerInput | UpdateMcpServerInput] }>()
const { t } = useI18n()

type AuthSelection = 'unchanged' | 'none' | 'bearer' | 'header'
const error = ref('')
const form = reactive({
  key: '',
  name: '',
  url: '',
  auth: 'none' as AuthSelection,
  headerName: '',
  connectTimeoutMs: 5000,
  requestTimeoutMs: 30000,
})

const credentialHint = computed(() => {
  const key = (form.key.trim() || props.server?.key || '').toUpperCase()
  return `VOICE_MCP_${key || '<SERVER_KEY>'}_TOKEN`
})

watch([open, () => props.server], () => {
  if (!open.value) return
  const existing = props.server
  form.key = existing?.key ?? ''
  form.name = existing?.name ?? ''
  form.url = existing?.url ?? ''
  form.auth = existing ? 'unchanged' : 'none'
  form.headerName = existing?.auth.type === 'header' ? existing.auth.header_name : ''
  form.connectTimeoutMs = existing?.connect_timeout_ms ?? 5000
  form.requestTimeoutMs = existing?.request_timeout_ms ?? 30000
  error.value = ''
}, { immediate: true })

function authentication(): McpAuthInput | undefined {
  if (form.auth === 'none') return { type: 'none' }
  if (form.auth === 'unchanged') return undefined
  if (form.auth === 'bearer') return { type: 'bearer' }
  const headerName = form.headerName.trim()
  if (!/^[a-z0-9-]+$/.test(headerName)) return undefined
  return { type: 'header', header_name: headerName }
}

function submit() {
  error.value = ''
  if (!props.server && !/^[a-z][a-z0-9_]{0,63}$/.test(form.key)) {
    error.value = t('mcp.validation.key')
    return
  }
  if (!form.name.trim() || !form.url.trim() ||
      !Number.isInteger(form.connectTimeoutMs) || form.connectTimeoutMs < 1 || form.connectTimeoutMs > 60000 ||
      !Number.isInteger(form.requestTimeoutMs) || form.requestTimeoutMs < 1 || form.requestTimeoutMs > 120000) {
    error.value = t('mcp.validation.fields')
    return
  }
  const auth = authentication()
  if (form.auth !== 'unchanged' && !auth) {
    error.value = t('mcp.validation.auth')
    return
  }
  const base = {
    name: form.name.trim(),
    url: form.url.trim(),
    connect_timeout_ms: form.connectTimeoutMs,
    request_timeout_ms: form.requestTimeoutMs,
  }
  if (props.server) {
    emit('save', { ...base, ...(auth ? { auth } : {}) })
  } else if (auth) {
    emit('save', { key: form.key.trim(), ...base, auth })
  }
}
</script>

<template>
  <BaseModal v-model="open" :title="server ? t('mcp.edit') : t('mcp.create')" :description="t('mcp.formHint')">
    <form class="space-y-4" @submit.prevent="submit">
      <p v-if="error || serverError" role="alert" class="rounded-lg border border-danger/40 p-3 text-sm text-danger">{{ error || serverError }}</p>
      <div class="grid gap-3 sm:grid-cols-2">
        <label v-if="!server" class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('mcp.key') }}</span>
          <input v-model="form.key" class="admin-input font-mono" placeholder="home_automation" required pattern="[a-z][a-z0-9_]{0,63}" maxlength="64" />
        </label>
        <label class="block space-y-1.5" :class="!server ? '' : 'sm:col-span-2'">
          <span class="text-sm font-medium">{{ t('mcp.name') }}</span>
          <input v-model="form.name" class="admin-input" maxlength="128" required />
        </label>
      </div>
      <label class="block space-y-1.5">
        <span class="text-sm font-medium">{{ t('mcp.endpoint') }}</span>
        <input v-model="form.url" class="admin-input font-mono text-xs" type="url" placeholder="https://mcp.example.com/mcp" required />
        <span class="block text-xs text-muted-foreground">{{ t('mcp.urlHint') }}</span>
      </label>
      <div class="grid gap-3 sm:grid-cols-2">
        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('mcp.connectTimeout') }}</span>
          <input v-model.number="form.connectTimeoutMs" type="number" min="1" max="60000" step="1" class="admin-input" />
        </label>
        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('mcp.requestTimeout') }}</span>
          <input v-model.number="form.requestTimeoutMs" type="number" min="1" max="120000" step="1" class="admin-input" />
        </label>
      </div>
      <div class="rounded-xl border border-border/70 p-4 space-y-3">
        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('mcp.auth') }}</span>
          <select v-model="form.auth" class="admin-input">
            <option v-if="server" value="unchanged">{{ t('mcp.authKeep') }}</option>
            <option value="none">{{ t('mcp.authNone') }}</option>
            <option value="bearer">Bearer (server environment)</option>
            <option value="header">Header (server environment)</option>
          </select>
        </label>
        <p v-if="server" class="text-xs text-muted-foreground">
          {{ t('mcp.authCurrent') }}: {{ server.auth.type }}
          {{ server.credential_env ? '— ' + server.credential_env : '' }}
        </p>
        <label v-if="form.auth === 'header'" class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('mcp.headerName') }}</span>
          <input v-model="form.headerName" class="admin-input font-mono" placeholder="x-api-key" />
        </label>
      </div>
      <p v-if="form.auth === 'bearer' || form.auth === 'header'" class="rounded-lg border border-border/70 p-3 text-xs text-muted-foreground">
        Token được cấp trên server qua <code>{{ credentialHint }}</code>.
        Web và SQLite không lưu hoặc đọc giá trị token.
      </p>
      <div class="flex justify-end gap-2">
        <Button type="button" variant="outline" :disabled="saving" @click="open = false">{{ t('common.cancel') }}</Button>
        <Button type="submit" :disabled="saving">{{ saving ? t('common.loading') : t('common.save') }}</Button>
      </div>
    </form>
  </BaseModal>
</template>

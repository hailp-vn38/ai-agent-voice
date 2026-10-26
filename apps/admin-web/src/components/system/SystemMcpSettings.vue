<script setup lang="ts">
import {
  AlertTriangle,
  Check,
  RefreshCcw,
  ShieldCheck,
  X,
} from '@lucide/vue'
import { computed, onMounted, ref } from 'vue'
import { systemMcpApi, type ChangeRequest, type SystemMcpSettings, type SystemMcpGrants } from '@/api/systemMcp'
import { isApiError } from '@/api/errors'
import { useI18n } from '@/composables/useI18n'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardHeader, CardTitle, CardContent, CardDescription } from '@/components/ui/card'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'

const { t, locale } = useI18n()
const isVi = computed(() => locale.value === 'vi')

const settings = ref<SystemMcpSettings>()
const requests = ref<ChangeRequest[]>([])
const error = ref('')
const busy = ref(false)
const selected = ref<ChangeRequest>()
const confirmOpen = ref(false)

const grantKeys: (keyof SystemMcpGrants)[] = [
  'system_read',
  'mcp_read',
  'mcp_discover',
  'mcp_manage_request',
]

const grantScopeLabel: Record<keyof SystemMcpGrants, string> = {
  system_read: 'system:read',
  mcp_read: 'mcp:read',
  mcp_discover: 'mcp:discover',
  mcp_manage_request: 'mcp:manage:request',
}

const grantTools: Record<keyof SystemMcpGrants, string[]> = {
  system_read: ['system_get_status', 'system_get_resources', 'system_get_logs'],
  mcp_read: ['mcp_list_servers', 'mcp_get_server', 'mcp_list_tools', 'mcp_get_agent_bindings'],
  mcp_discover: ['mcp_discover_tools'],
  mcp_manage_request: [
    'mcp_request_server_change',
    'mcp_request_agent_binding_change',
    'mcp_request_tool_review',
  ],
}

const activeGrantsCount = computed(() => {
  if (!settings.value) return 0
  return grantKeys.filter((key) => settings.value?.grants[key]).length
})

const pendingRequestsCount = computed(
  () => requests.value.filter((item) => item.status === 'pending').length,
)

function statusBadgeVariant(
  status: ChangeRequest['status'],
): 'default' | 'secondary' | 'outline' | 'success' | 'danger' {
  if (status === 'applied') return 'success'
  if (status === 'failed' || status === 'rejected') return 'danger'
  if (status === 'pending' || status === 'claimed') return 'default'
  return 'secondary'
}

function failure(e: unknown) {
  error.value = isApiError(e) ? e.code : 'connection_failed'
}

async function load() {
  busy.value = true
  error.value = ''
  try {
    const [saved, proposals] = await Promise.all([
      systemMcpApi.settings(),
      systemMcpApi.requests(),
    ])
    settings.value = saved
    requests.value = proposals
  } catch (e) {
    failure(e)
  } finally {
    busy.value = false
  }
}

async function save() {
  if (!settings.value) return
  busy.value = true
  error.value = ''
  try {
    settings.value = await systemMcpApi.save(settings.value.grants, settings.value.revision)
  } catch (e) {
    await load()
    failure(e)
  } finally {
    busy.value = false
  }
}

function select(request: ChangeRequest) {
  selected.value = request
  confirmOpen.value = true
}

async function decide(request: ChangeRequest, confirm: boolean) {
  busy.value = true
  error.value = ''
  try {
    const updated = await (confirm
      ? systemMcpApi.confirm(request.id)
      : systemMcpApi.reject(request.id))
    requests.value = requests.value.map((item) => (item.id === updated.id ? updated : item))
  } catch (e) {
    await load()
    failure(e)
  } finally {
    busy.value = false
  }
}

onMounted(load)
</script>

<template>
  <Card>
    <CardHeader>
      <div class="flex flex-wrap items-start justify-between gap-3">
        <div>
          <CardTitle class="flex items-center gap-2">
            <ShieldCheck class="size-5 text-studio-violet" />
            <span>System MCP</span>
          </CardTitle>
          <CardDescription class="mt-1">
            {{ t('systemMcp.description') }}
          </CardDescription>
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <Badge v-if="settings" :variant="settings.enabled ? 'success' : 'secondary'">
            {{ settings.enabled ? t('systemMcp.enabled') : t('systemMcp.off') }}
          </Badge>
          <Badge v-if="settings" variant="outline">
            {{ activeGrantsCount }} / {{ grantKeys.length }} Grants
          </Badge>
          <Button type="button" variant="outline" size="sm" :disabled="busy" @click="load">
            <RefreshCcw :class="['size-4', busy && 'animate-spin']" />
            <span>{{ t('common.refresh') }}</span>
          </Button>
        </div>
      </div>
    </CardHeader>

    <CardContent class="space-y-5">
      <div
        v-if="error"
        role="alert"
        class="rounded-lg border border-destructive/40 bg-destructive/5 p-3 text-sm text-destructive"
      >
        {{ t('assistant.error') }}: {{ error }}
      </div>

      <div
        v-if="settings && !settings.enabled"
        class="flex items-start gap-2.5 rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-sm text-amber-800 dark:text-amber-300"
      >
        <AlertTriangle class="mt-0.5 size-4 shrink-0 text-amber-500" />
        <span>{{ t('systemMcp.disabled') }}</span>
      </div>

      <div class="grid gap-6 lg:grid-cols-2">
        <!-- Studio Assistant Grants Form -->
        <div class="space-y-3">
          <div class="flex items-center justify-between gap-2">
            <h3 class="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              {{ isVi ? 'Quyền công cụ Studio Assistant' : 'Studio Assistant Tool Grants' }}
            </h3>
            <span v-if="settings" class="text-xs text-muted-foreground">
              Revision: <code>{{ settings.revision }}</code>
            </span>
          </div>

          <form v-if="settings" class="space-y-3" @submit.prevent="save">
            <div class="space-y-2.5">
              <label
                v-for="key in grantKeys"
                :key="key"
                :class="[
                  'flex items-start gap-3 rounded-lg border border-border p-3 transition-colors',
                  busy || !settings.enabled
                    ? 'cursor-not-allowed bg-muted/10 opacity-60'
                    : 'cursor-pointer hover:bg-muted/30',
                ]"
              >
                <input
                  v-model="settings.grants[key]"
                  type="checkbox"
                  :disabled="busy || !settings.enabled"
                  class="mt-1"
                />
                <div class="min-w-0 flex-1 space-y-1.5 text-sm">
                  <div class="flex flex-wrap items-center justify-between gap-2">
                    <span class="font-medium">{{ t(`systemMcp.${key}`) }}</span>
                    <code class="rounded bg-muted px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground">
                      {{ grantScopeLabel[key] }}
                    </code>
                  </div>
                  <div class="flex flex-wrap gap-1">
                    <code
                      v-for="toolName in grantTools[key]"
                      :key="toolName"
                      class="rounded border border-border/70 bg-background/80 px-1.5 py-0.5 font-mono text-[11px] text-studio-violet"
                    >
                      {{ toolName }}
                    </code>
                  </div>
                </div>
              </label>
            </div>

            <p class="rounded-lg border border-border/60 bg-muted/20 p-2.5 text-xs text-muted-foreground">
              {{ t('systemMcp.reconnect') }}
            </p>

            <div class="flex flex-wrap items-center justify-between gap-3 pt-1">
              <span class="text-xs text-muted-foreground">
                Revision: <code>{{ settings.revision }}</code>
              </span>
              <Button type="submit" :disabled="busy || !settings.enabled">
                <ShieldCheck class="size-4" />
                <span>{{ t('common.save') }}</span>
              </Button>
            </div>
          </form>
        </div>

        <!-- Change Requests Panel -->
        <div class="space-y-3">
          <div class="flex items-center justify-between gap-2">
            <h3 class="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              {{ t('systemMcp.requests') }} ({{ requests.length }})
            </h3>
            <Badge :variant="pendingRequestsCount > 0 ? 'default' : 'secondary'">
              {{ pendingRequestsCount }} {{ t('systemMcp.status.pending') }}
            </Badge>
          </div>

          <p
            v-if="!requests.length"
            class="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground"
          >
            {{ t('systemMcp.empty') }}
          </p>

          <div v-else class="space-y-3">
            <article
              v-for="request in requests"
              :key="request.id"
              class="space-y-3 rounded-lg border border-border bg-muted/15 p-3.5 text-sm"
            >
              <div class="flex flex-wrap items-center justify-between gap-2 border-b border-border/60 pb-2">
                <code class="font-mono text-xs font-bold text-studio-violet">
                  {{ request.command.tool }}
                </code>
                <Badge :variant="statusBadgeVariant(request.status)">
                  {{ t(`systemMcp.status.${request.status}`) }}
                </Badge>
              </div>

              <dl class="grid gap-1.5 text-xs sm:text-sm">
                <div><dt class="inline font-medium text-muted-foreground">{{ t('systemMcp.target') }}: </dt><dd class="inline break-all font-mono font-semibold">{{ request.command.server_key }}<template v-if="'agent_key' in request.command"> / {{ request.command.agent_key }}</template><template v-if="'tool_name' in request.command"> / {{ request.command.tool_name }}</template></dd></div>
                <div><dt class="inline font-medium text-muted-foreground">{{ t('systemMcp.effect') }}: </dt><dd class="inline font-medium"><template v-if="'enabled' in request.command">{{ request.command.enabled ? t('systemMcp.enabled') : t('systemMcp.off') }}</template><template v-else>{{ t('systemMcp.allow') }}: {{ request.command.allow }}; {{ t('systemMcp.sensitive') }}: {{ request.command.sensitive }}</template></dd></div>
                <div><dt class="inline font-medium text-muted-foreground">{{ t('systemMcp.revision') }}: </dt><dd class="inline font-mono">{{ 'expected_agent_revision' in request.command ? request.command.expected_agent_revision : request.command.expected_revision }}<template v-if="'observed_revision' in request.command">; {{ t('systemMcp.observation') }}: {{ request.command.observed_revision }}; {{ request.command.fingerprint }}</template></dd></div>
                <div><dt class="inline font-medium text-muted-foreground">{{ t('systemMcp.expires') }}: </dt><dd class="inline">{{ new Date(request.expires_at * 1000).toLocaleString() }}</dd></div>
                <div><dt class="inline font-medium text-muted-foreground">{{ t('systemMcp.status') }}: </dt><dd class="inline font-medium">{{ t(`systemMcp.status.${request.status}`) }}</dd></div>
              </dl>

              <div v-if="request.status === 'pending'" class="flex flex-wrap gap-2 pt-1">
                <Button size="sm" :disabled="busy" @click="select(request)">
                  <Check class="size-4" />
                  <span>{{ t('common.confirm') }}</span>
                </Button>
                <Button size="sm" variant="outline" :disabled="busy" @click="decide(request, false)">
                  <X class="size-4" />
                  <span>{{ t('systemMcp.reject') }}</span>
                </Button>
              </div>
            </article>
          </div>
        </div>
      </div>
    </CardContent>
  </Card>
  <ConfirmDialog
    v-model="confirmOpen"
    :title="t('systemMcp.confirmTitle')"
    :description="t('systemMcp.confirmDescription')"
    @confirm="selected && decide(selected, true)"
  />
</template>

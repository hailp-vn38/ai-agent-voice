<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { systemMcpApi, type ChangeRequest, type SystemMcpSettings, type SystemMcpGrants } from '@/api/systemMcp'
import { isApiError } from '@/api/errors'
import { useI18n } from '@/composables/useI18n'
import { Button } from '@/components/ui/button'
import { Card, CardHeader, CardTitle, CardContent, CardDescription } from '@/components/ui/card'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
const { t } = useI18n()
const settings = ref<SystemMcpSettings>()
const requests = ref<ChangeRequest[]>([])
const error = ref('')
const busy = ref(false)
const selected = ref<ChangeRequest>()
const confirmOpen = ref(false)
const grantKeys: (keyof SystemMcpGrants)[] = ['system_read', 'mcp_read', 'mcp_discover', 'mcp_manage_request']
function failure(e: unknown) { error.value = isApiError(e) ? e.code : 'connection_failed' }
async function load() {
  busy.value = true
  error.value = ''
  try { const [saved, proposals] = await Promise.all([systemMcpApi.settings(), systemMcpApi.requests()]); settings.value = saved; requests.value = proposals }
  catch (e) { failure(e) }
  finally { busy.value = false }
}
async function save() {
  if (!settings.value) return
  busy.value = true
  error.value = ''
  try { settings.value = await systemMcpApi.save(settings.value.grants, settings.value.revision) }
  catch (e) { await load(); failure(e) }
  finally { busy.value = false }
}
function select(request: ChangeRequest) { selected.value = request; confirmOpen.value = true }
async function decide(request: ChangeRequest, confirm: boolean) {
  busy.value = true
  error.value = ''
  try {
    const updated = await (confirm ? systemMcpApi.confirm(request.id) : systemMcpApi.reject(request.id))
    requests.value = requests.value.map(item => item.id === updated.id ? updated : item)
  } catch (e) { await load(); failure(e) }
  finally { busy.value = false }
}
onMounted(load)
</script>
<template>
  <Card>
    <CardHeader><CardTitle>System MCP</CardTitle><CardDescription>{{ t('systemMcp.description') }}</CardDescription></CardHeader>
    <CardContent class="space-y-4">
      <p v-if="error" role="alert" class="text-destructive">{{ t('assistant.error') }}: {{ error }}</p>
      <p v-if="settings && !settings.enabled">{{ t('systemMcp.disabled') }}</p>
      <form v-if="settings" class="space-y-3" @submit.prevent="save">
        <label v-for="key in grantKeys" :key="key" class="flex items-center gap-2"><input v-model="settings.grants[key]" type="checkbox" :disabled="busy || !settings.enabled" />{{ t(`systemMcp.${key}`) }}</label>
        <p class="text-sm text-muted-foreground">{{ t('systemMcp.reconnect') }}</p>
        <Button type="submit" :disabled="busy || !settings.enabled">{{ t('common.save') }}</Button>
      </form>
      <div class="flex items-center justify-between"><h3 class="font-medium">{{ t('systemMcp.requests') }}</h3><Button variant="outline" :disabled="busy" @click="load">{{ t('common.refresh') }}</Button></div>
      <p v-if="!requests.length">{{ t('systemMcp.empty') }}</p>
      <article v-for="request in requests" :key="request.id" class="space-y-2 rounded border p-3 text-sm">
        <dl class="grid gap-1">
          <div><dt class="inline font-medium">{{ t('systemMcp.target') }}: </dt><dd class="inline break-all">{{ request.command.server_key }}<template v-if="'agent_key' in request.command"> / {{ request.command.agent_key }}</template><template v-if="'tool_name' in request.command"> / {{ request.command.tool_name }}</template></dd></div>
          <div><dt class="inline font-medium">{{ t('systemMcp.effect') }}: </dt><dd class="inline"><template v-if="'enabled' in request.command">{{ request.command.enabled ? t('systemMcp.enabled') : t('systemMcp.off') }}</template><template v-else>{{ t('systemMcp.allow') }}: {{ request.command.allow }}; {{ t('systemMcp.sensitive') }}: {{ request.command.sensitive }}</template></dd></div>
          <div><dt class="inline font-medium">{{ t('systemMcp.revision') }}: </dt><dd class="inline">{{ 'expected_agent_revision' in request.command ? request.command.expected_agent_revision : request.command.expected_revision }}<template v-if="'observed_revision' in request.command">; {{ t('systemMcp.observation') }}: {{ request.command.observed_revision }}; {{ request.command.fingerprint }}</template></dd></div>
          <div><dt class="inline font-medium">{{ t('systemMcp.expires') }}: </dt><dd class="inline">{{ new Date(request.expires_at * 1000).toLocaleString() }}</dd></div>
          <div><dt class="inline font-medium">{{ t('systemMcp.status') }}: </dt><dd class="inline">{{ t(`systemMcp.status.${request.status}`) }}</dd></div>
        </dl>
        <div v-if="request.status === 'pending'" class="flex gap-2"><Button :disabled="busy" @click="select(request)">{{ t('common.confirm') }}</Button><Button variant="outline" :disabled="busy" @click="decide(request, false)">{{ t('systemMcp.reject') }}</Button></div>
      </article>
    </CardContent>
  </Card>
  <ConfirmDialog v-model="confirmOpen" :title="t('systemMcp.confirmTitle')" :description="t('systemMcp.confirmDescription')" @confirm="selected && decide(selected, true)" />
</template>

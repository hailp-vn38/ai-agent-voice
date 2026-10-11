<script setup lang="ts">
import SystemMcpSettings from './SystemMcpSettings.vue'
import { computed, onMounted, ref, watch } from 'vue'
import { assistantApi, type AssistantSettings } from '@/api/assistant'
import { isApiError } from '@/api/errors'
import { useAdminStore } from '@/stores/admin'
import { useAssistantStore } from '@/stores/assistant'
import { useI18n } from '@/composables/useI18n'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'

const admin = useAdminStore()
const assistant = useAssistantStore()
const { t } = useI18n()
const saved = ref<AssistantSettings>()
const agent = ref('')
const template = ref('')
const enabled = ref(false)
const loading = ref(true)
const saving = ref(false)
const error = ref('')
const confirmDisable = ref(false)
const templates = computed(() => admin.getTemplatesForAgent(agent.value))
const effectiveTemplate = computed(() => template.value ? admin.getTemplate(template.value) : admin.getDefaultTemplate(agent.value))
const dirty = computed(() => !saved.value?.configured || saved.value.agent_key !== agent.value || (saved.value.template_key ?? '') !== template.value || saved.value.enabled !== enabled.value)
watch(agent, () => { if (!templates.value.some(item => item.id === template.value)) template.value = '' })
async function load() {
  loading.value = true
  try {
    saved.value = await assistantApi.settings()
    agent.value = saved.value.agent_key ?? ''
    template.value = saved.value.template_key ?? ''
    enabled.value = saved.value.enabled
  } catch (e) { error.value = isApiError(e) ? e.code : 'connection_failed' }
  finally { loading.value = false }
}
onMounted(load)
async function save() {
  saving.value = true
  error.value = ''
  confirmDisable.value = false
  try {
    saved.value = await assistantApi.save({ agent_key: agent.value, template_key: template.value || null, enabled: enabled.value }, saved.value?.revision ?? undefined)
  } catch (e) {
    const code = isApiError(e) ? e.code : 'connection_failed'
    if (code === 'revision_conflict') await load()
    error.value = code
  } finally { saving.value = false }
}
async function copyDevice() {
  try { if (saved.value?.device_id) await navigator.clipboard.writeText(saved.value.device_id) }
  catch { error.value = 'clipboard_unavailable' }
}
function submit() { if (saved.value?.enabled && !enabled.value) confirmDisable.value = true; else void save() }
</script>

<template>
  <Card>
    <CardHeader>
      <CardTitle>{{ t('assistant.title') }}</CardTitle>
      <CardDescription>{{ t('assistant.settingsDescription') }}</CardDescription>
    </CardHeader>
    <CardContent>
      <p v-if="loading" role="status">{{ t('common.loading') }}</p>
      <form v-else class="space-y-4" @submit.prevent="submit">
        <p v-if="error" role="alert" class="text-destructive">{{ t('assistant.error') }}: {{ error }}</p>
        <label class="flex items-center gap-2"><input v-model="enabled" type="checkbox" :disabled="saving" />{{ t('assistant.enabled') }}</label>
        <div class="grid gap-4 sm:grid-cols-2">
          <label class="space-y-1"><span>{{ t('assistant.agent') }}</span>
            <select v-model="agent" class="admin-input w-full" :disabled="saving" required>
              <option value="" disabled>{{ t('assistant.selectAgent') }}</option>
              <option v-for="item in admin.agents" :key="item.id" :value="item.id">{{ item.name }}</option>
            </select>
          </label>
          <label class="space-y-1"><span>{{ t('assistant.template') }}</span>
            <select v-model="template" class="admin-input w-full" :disabled="saving || !agent">
              <option value="">{{ t('assistant.agentDefault') }}</option>
              <option v-for="item in templates" :key="item.id" :value="item.id">{{ item.name }}</option>
            </select>
          </label>
        </div>
        <div v-if="effectiveTemplate" class="flex flex-wrap gap-3 text-sm">
          <template v-for="kind in (['vad', 'asr', 'llm', 'tts'] as const)" :key="kind">
            <RouterLink v-if="effectiveTemplate.providerBindings[kind]" class="underline" :to="`/providers/${effectiveTemplate.providerBindings[kind]}`">{{ kind.toUpperCase() }}: {{ admin.getProvider(effectiveTemplate.providerBindings[kind])?.name ?? effectiveTemplate.providerBindings[kind] }}</RouterLink>
            <span v-else>{{ kind.toUpperCase() }}: {{ t('common.unknown') }}</span>
          </template>
        </div>
        <p class="break-all text-sm">{{ t('assistant.managedDevice') }}: <code>{{ saved?.device_id ?? '—' }}</code> <Button v-if="saved?.device_id" type="button" variant="ghost" size="sm" @click="copyDevice">{{ t('deviceCard.copyId') }}</Button></p>
        <p class="text-sm">{{ t('assistant.connections') }}: {{ saved?.active_connections ?? 0 }}</p>
        <div class="flex flex-wrap gap-2">
          <Button type="submit" :disabled="saving || !agent || !dirty">{{ t('common.save') }}</Button>
          <Button type="button" variant="outline" :disabled="dirty || !saved?.enabled || saving" @click="assistant.open()">{{ t('assistant.open') }}</Button>
          <Button type="button" variant="ghost" :disabled="saving" @click="load">{{ t('common.retry') }}</Button>
        </div>
      </form>
    </CardContent>
  </Card>
  <SystemMcpSettings />
  <ConfirmDialog v-model="confirmDisable" :title="t('assistant.disableTitle')" :description="t('assistant.disableDescription')" @confirm="save" />
</template>

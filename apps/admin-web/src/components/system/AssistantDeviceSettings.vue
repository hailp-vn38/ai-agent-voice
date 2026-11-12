<script setup lang="ts">
import { ArrowUpRight, Bot, Copy, Radio, Save, ShieldCheck } from '@lucide/vue'
import { computed, onMounted, ref, watch } from 'vue'
import { RouterLink } from 'vue-router'

import { assistantApi, type AssistantSettings } from '@/api/assistant'
import { isApiError } from '@/api/errors'
import type { MemoryMode } from '@/api/types/devices'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useAdminStore } from '@/stores/admin'
import { useAssistantStore } from '@/stores/assistant'

const admin = useAdminStore()
const assistant = useAssistantStore()
const { t } = useI18n()

const saved = ref<AssistantSettings>()
const agent = ref('')
const template = ref('')
const enabled = ref(false)
const memoryMode = ref<MemoryMode>('off')
const loading = ref(true)
const saving = ref(false)
const error = ref('')
const confirmDisable = ref(false)

const templates = computed(() => admin.getTemplatesForAgent(agent.value))
const effectiveTemplate = computed(() =>
  template.value ? admin.getTemplate(template.value) : admin.getDefaultTemplate(agent.value),
)
const dirty = computed(
  () =>
    !saved.value?.configured ||
    saved.value.agent_key !== agent.value ||
    (saved.value.template_key ?? '') !== template.value ||
    saved.value.enabled !== enabled.value ||
    (saved.value.memory_mode ?? 'off') !== memoryMode.value,
)

watch(agent, () => {
  if (!templates.value.some(item => item.id === template.value)) template.value = ''
})

async function load() {
  loading.value = true
  error.value = ''
  try {
    saved.value = await assistantApi.settings()
    agent.value = saved.value.agent_key ?? ''
    template.value = saved.value.template_key ?? ''
    enabled.value = saved.value.enabled
    memoryMode.value = saved.value.memory_mode ?? 'off'
  } catch (e) {
    error.value = isApiError(e) ? e.code : 'connection_failed'
  } finally {
    loading.value = false
  }
}

onMounted(load)

async function save() {
  if (saving.value || loading.value || !agent.value) return
  saving.value = true
  error.value = ''
  confirmDisable.value = false
  try {
    saved.value = await assistantApi.save(
      {
        agent_key: agent.value,
        template_key: template.value || null,
        enabled: enabled.value,
        memory_mode: memoryMode.value,
      },
      saved.value?.revision ?? undefined,
    )
  } catch (e) {
    const code = isApiError(e) ? e.code : 'connection_failed'
    if (code === 'revision_conflict') await load()
    error.value = code
  } finally {
    saving.value = false
  }
}

async function copyDevice() {
  try {
    if (saved.value?.device_id) await navigator.clipboard.writeText(saved.value.device_id)
  } catch {
    error.value = 'clipboard_unavailable'
  }
}

function submit() {
  if (saved.value?.enabled && !enabled.value) confirmDisable.value = true
  else void save()
}

const busy = computed(() => loading.value || saving.value)
defineExpose({ reload: load, busy })
</script>

<template>
  <div class="assistant-workspace">
    <section class="sys-panel assistant-configuration">
      <header class="sys-panel-header">
        <span class="sys-section-number">01</span>
        <div><h2>{{ t('system.detail.binding') }}</h2><p>{{ t('system.detail.bindingDescription') }}</p></div>
      </header>
      <p v-if="loading" role="status" class="sys-loading">{{ t('common.loading') }}</p>
      <form v-else class="assistant-form" @submit.prevent="submit">
        <p v-if="error" role="alert" class="sys-alert">{{ t('assistant.error') }}: {{ error }}</p>
        <label class="assistant-enable">
          <div><span>{{ t('system.detail.assistantEnabled') }}</span><p>{{ t('system.detail.enableDescription') }}</p></div>
          <input v-model="enabled" type="checkbox" class="sys-toggle-input" :disabled="saving" />
        </label>
        <div class="sys-form-grid">
          <label class="sys-field"><span>{{ t('assistant.agent') }}</span>
            <select v-model="agent" class="admin-input" :disabled="saving" required>
              <option value="" disabled>{{ t('assistant.selectAgent') }}</option>
              <option v-for="item in admin.agents" :key="item.id" :value="item.id">{{ item.name }}</option>
            </select>
          </label>
          <label class="sys-field"><span>{{ t('assistant.template') }}</span>
            <select v-model="template" class="admin-input" :disabled="saving || !agent">
              <option value="">{{ t('assistant.agentDefault') }}</option>
              <option v-for="item in templates" :key="item.id" :value="item.id">{{ item.name }}</option>
            </select>
          </label>
        </div>
        <section v-if="effectiveTemplate" class="assistant-pipeline">
          <div class="sys-small-heading"><h3>{{ t('system.detail.pipeline') }}</h3><p>{{ t('system.detail.pipelineDescription') }}</p></div>
          <div class="pipeline-nodes">
            <div v-for="kind in (['vad', 'asr', 'llm', 'tts'] as const)" :key="kind" class="pipeline-node">
              <span>{{ kind.toUpperCase() }}</span>
              <RouterLink v-if="effectiveTemplate.providerBindings[kind]" :to="`/providers/${effectiveTemplate.providerBindings[kind]}`">
                {{ admin.getProvider(effectiveTemplate.providerBindings[kind])?.name ?? effectiveTemplate.providerBindings[kind] }}<ArrowUpRight class="size-3" aria-hidden="true" />
              </RouterLink>
              <span v-else class="pipeline-unknown">{{ t('common.unknown') }}</span>
            </div>
          </div>
        </section>
        <label class="sys-field assistant-memory"><span>{{ t('system.detail.memory') }}</span>
          <select v-model="memoryMode" data-assistant-memory-mode class="admin-input" :disabled="saving">
            <option value="off">{{ t('memoryV3.mode.off') }}</option>
            <option value="read">{{ t('memoryV3.mode.read') }}</option>
            <option value="read_write">{{ t('memoryV3.mode.read_write') }}</option>
          </select>
          <small>{{ t('memoryV3.modeHint') }} {{ t('memoryV3.privateNotice') }}</small>
        </label>
        <footer class="assistant-form-footer">
          <span class="assistant-save-state" :data-dirty="dirty"><span />{{ t(dirty ? 'system.detail.pendingChanges' : 'system.detail.saved') }}</span>
          <div class="sys-actions">
            <Button type="button" variant="ghost" size="sm" :disabled="saving" @click="load">{{ t(dirty ? 'system.detail.discardChanges' : 'system.refresh') }}</Button>
            <Button type="submit" class="sys-primary" :disabled="saving || !agent || !dirty"><Save class="size-3.5" aria-hidden="true" />{{ t('common.save') }}</Button>
          </div>
        </footer>
      </form>
    </section>

    <aside class="assistant-identity">
      <div class="identity-card" :data-connected="!!saved?.enabled && (saved?.active_connections ?? 0) > 0">
        <div class="identity-caption"><span>{{ t('system.detail.identity') }}</span><Radio class="size-4" aria-hidden="true" /></div>
        <div class="identity-avatar" aria-hidden="true"><span /><span /><Bot class="size-12" /></div>
        <h2>{{ saved?.device_id ?? '—' }}</h2>
        <p class="identity-connection" data-testid="assistant-connection-state"><span />{{ !saved ? t('common.unknown') : !saved.enabled ? t('system.deck.disabled') : t(saved.active_connections > 0 ? 'system.detail.connected' : 'system.detail.disconnected') }}</p>
        <dl class="identity-facts">
          <div><dt>{{ t('system.detail.connections') }}</dt><dd>{{ saved?.active_connections ?? '—' }}</dd></div>
          <div><dt>{{ t('system.detail.assignedAgent') }}</dt><dd>{{ saved?.agent_key ?? '—' }}</dd></div>
          <div><dt>{{ t('system.hub.assignedTemplate') }}</dt><dd>{{ saved ? saved.template_key || t('assistant.agentDefault') : '—' }}</dd></div>
        </dl>
        <button v-if="saved?.device_id" type="button" class="identity-copy" @click="copyDevice"><Copy class="size-3.5" aria-hidden="true" />{{ t('deviceCard.copyId') }}</button>
      </div>
      <div class="assistant-launch">
        <ShieldCheck class="size-4" aria-hidden="true" />
        <p>{{ t('system.detail.savedBinding') }}</p>
        <Button type="button" variant="outline" class="w-full" :disabled="dirty || !saved?.enabled || busy" @click="assistant.open()">
          {{ t('system.detail.tryAssistant') }}<ArrowUpRight class="size-3.5" aria-hidden="true" />
        </Button>
      </div>
    </aside>
  </div>
  <ConfirmDialog v-model="confirmDisable" :title="t('assistant.disableTitle')" :description="t('assistant.disableDescription')" @confirm="save" />
</template>

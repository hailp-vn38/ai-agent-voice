<script setup lang="ts">
import { Bot, Copy, Wifi } from '@lucide/vue'
import { computed, onMounted, ref, watch } from 'vue'
import { RouterLink } from 'vue-router'

import { assistantApi, type AssistantSettings } from '@/api/assistant'
import { isApiError } from '@/api/errors'
import type { MemoryMode } from '@/api/types/devices'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import { Badge } from '@/components/ui/badge'
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

defineExpose({ reload: load })
</script>

<template>
  <div class="grid items-start gap-4 lg:grid-cols-[minmax(0,1.5fr)_minmax(260px,0.75fr)]">
    <!-- Left Column: Assistant Device Configuration -->
    <section class="studio-panel p-4">
      <div class="mb-4 flex flex-wrap items-center justify-between gap-3">
        <div class="flex items-center gap-2.5">
          <span
            class="grid size-9 shrink-0 place-items-center rounded-xl bg-studio-violet/12 text-studio-violet"
          >
            <Bot class="size-4.5" aria-hidden="true" />
          </span>
          <div>
            <h2 class="text-sm font-semibold">{{ t('assistant.title') }}</h2>
            <p class="text-xs text-muted-foreground">{{ t('assistant.settingsDescription') }}</p>
          </div>
        </div>
        <Badge :variant="enabled ? 'success' : 'outline'">
          {{ enabled ? 'Enabled' : 'Disabled' }}
        </Badge>
      </div>

      <p v-if="loading" role="status" class="py-4 text-xs text-muted-foreground">
        {{ t('common.loading') }}
      </p>

      <form v-else class="space-y-4" @submit.prevent="submit">
        <p v-if="error" role="alert" class="text-xs text-destructive">
          {{ t('assistant.error') }}: {{ error }}
        </p>

        <label
          class="flex cursor-pointer items-center justify-between gap-3 rounded-xl border border-border/70 bg-surface/60 px-3.5 py-2.5"
        >
          <div>
            <span class="block text-xs font-semibold">{{ t('assistant.enabled') }}</span>
            <span class="block text-[11px] text-muted-foreground">
              {{ t('assistant.disableDescription') }}
            </span>
          </div>
          <input
            v-model="enabled"
            type="checkbox"
            class="size-4 accent-primary"
            :disabled="saving"
          />
        </label>

        <div class="grid gap-3.5 sm:grid-cols-2">
          <label class="block space-y-1.5 text-xs">
            <span class="font-semibold">{{ t('assistant.agent') }}</span>
            <select
              v-model="agent"
              class="admin-input h-9 w-full text-xs"
              :disabled="saving"
              required
            >
              <option value="" disabled>{{ t('assistant.selectAgent') }}</option>
              <option v-for="item in admin.agents" :key="item.id" :value="item.id">
                {{ item.name }}
              </option>
            </select>
          </label>

          <label class="block space-y-1.5 text-xs">
            <span class="font-semibold">{{ t('assistant.template') }}</span>
            <select
              v-model="template"
              class="admin-input h-9 w-full text-xs"
              :disabled="saving || !agent"
            >
              <option value="">{{ t('assistant.agentDefault') }}</option>
              <option v-for="item in templates" :key="item.id" :value="item.id">
                {{ item.name }}
              </option>
            </select>
          </label>
        </div>

        <label class="block space-y-1.5 text-xs">
          <span class="font-semibold">{{ t('memoryV3.mode') }}</span>
          <select v-model="memoryMode" data-assistant-memory-mode class="admin-input h-9 w-full text-xs" :disabled="saving">
            <option value="off">{{ t('memoryV3.mode.off') }}</option>
            <option value="read">{{ t('memoryV3.mode.read') }}</option>
            <option value="read_write">{{ t('memoryV3.mode.read_write') }}</option>
          </select>
          <span class="block text-[11px] text-muted-foreground">{{ t('memoryV3.modeHint') }} {{ t('memoryV3.privateNotice') }}</span>
        </label>

        <div v-if="effectiveTemplate" class="flex flex-wrap gap-2.5 text-xs">
          <template v-for="kind in (['vad', 'asr', 'llm', 'tts'] as const)" :key="kind">
            <RouterLink
              v-if="effectiveTemplate.providerBindings[kind]"
              class="underline hover:text-studio-violet"
              :to="`/providers/${effectiveTemplate.providerBindings[kind]}`"
            >
              {{ kind.toUpperCase() }}:
              {{
                admin.getProvider(effectiveTemplate.providerBindings[kind])?.name ??
                effectiveTemplate.providerBindings[kind]
              }}
            </RouterLink>
            <span v-else class="text-muted-foreground">
              {{ kind.toUpperCase() }}: {{ t('common.unknown') }}
            </span>
          </template>
        </div>

        <div class="flex flex-wrap justify-end gap-2 border-t border-border/60 pt-3">
          <Button type="button" variant="ghost" size="sm" :disabled="saving" @click="load">
            {{ t('common.retry') }}
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            :disabled="dirty || !saved?.enabled || saving"
            @click="assistant.open()"
          >
            {{ t('assistant.open') }}
          </Button>
          <Button type="submit" size="sm" :disabled="saving || !agent || !dirty">
            {{ t('common.save') }}
          </Button>
        </div>
      </form>
    </section>

    <!-- Right Column: Managed Device Inspector -->
    <aside class="studio-panel p-4">
      <div class="mb-3 flex items-center gap-2.5">
        <span
          class="grid size-8 shrink-0 place-items-center rounded-lg bg-studio-cyan/12 text-studio-cyan"
        >
          <Wifi class="size-4" aria-hidden="true" />
        </span>
        <div>
          <h3 class="text-xs font-semibold">{{ t('system.hub.managedDeviceTitle') }}</h3>
          <p class="text-[11px] text-muted-foreground">{{ t('system.hub.managedDeviceSub') }}</p>
        </div>
      </div>

      <div class="divide-y divide-border/60 text-xs">
        <div class="flex items-center justify-between gap-2 py-2.5">
          <span class="text-muted-foreground">{{ t('assistant.managedDevice') }}</span>
          <code class="font-mono font-semibold">{{ saved?.device_id ?? '—' }}</code>
        </div>
        <div class="flex items-center justify-between gap-2 py-2.5">
          <span class="text-muted-foreground">{{ t('assistant.connections') }}</span>
          <strong class="font-mono">{{ saved?.active_connections ?? 0 }}</strong>
        </div>
        <div class="flex items-center justify-between gap-2 py-2.5">
          <span class="text-muted-foreground">{{ t('system.hub.connectionState') }}</span>
          <strong>{{ saved?.enabled ? 'Connected' : 'Disabled' }}</strong>
        </div>
        <div class="flex items-center justify-between gap-2 py-2.5">
          <span class="text-muted-foreground">{{ t('system.hub.assignedAgent') }}</span>
          <strong class="font-mono">{{ agent || saved?.agent_key || '—' }}</strong>
        </div>
        <div class="flex items-center justify-between gap-2 py-2.5">
          <span class="text-muted-foreground">{{ t('system.hub.assignedTemplate') }}</span>
          <strong class="font-mono">{{ template || saved?.template_key || t('assistant.agentDefault') }}</strong>
        </div>
      </div>

      <Button
        v-if="saved?.device_id"
        type="button"
        variant="outline"
        size="sm"
        class="mt-3 w-full text-xs"
        @click="copyDevice"
      >
        <Copy class="size-3.5" aria-hidden="true" />
        {{ t('deviceCard.copyId') }}
      </Button>
    </aside>
  </div>

  <ConfirmDialog
    v-model="confirmDisable"
    :title="t('assistant.disableTitle')"
    :description="t('assistant.disableDescription')"
    @confirm="save"
  />
</template>

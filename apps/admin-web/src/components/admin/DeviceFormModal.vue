<script setup lang="ts">
import { computed, reactive, watch } from 'vue'

import BaseModal from '@/components/admin/BaseModal.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { Agent, AgentTemplate, Device, DeviceStatus } from '@/domain/admin'

const props = defineProps<{
  device?: Device
  agent: Agent
  templates: AgentTemplate[]
}>()

const open = defineModel<boolean>({ required: true })

const emit = defineEmits<{
  save: [payload: {
    name: string
    deviceId: string
    description: string
    status: DeviceStatus
    templateId?: string
  }]
}>()

const { t } = useI18n()

type TemplateMode = 'default' | 'override'

const form = reactive({
  name: '',
  deviceId: '',
  description: '',
  status: 'offline' as DeviceStatus,
  templateMode: 'default' as TemplateMode,
  templateId: '',
})

const defaultTemplate = computed(() =>
  props.templates.find((template) => template.id === props.agent.defaultTemplateId),
)

watch(
  () => [open.value, props.device] as const,
  () => {
    if (!open.value) return
    form.name = props.device?.name ?? ''
    form.deviceId = props.device?.deviceId ?? ''
    form.description = props.device?.description ?? ''
    form.status = props.device?.status ?? 'offline'
    form.templateMode = props.device?.templateId ? 'override' : 'default'
    form.templateId = props.device?.templateId ?? props.templates[0]?.id ?? ''
  },
  { immediate: true },
)

function submit() {
  if (!form.name.trim() || !form.deviceId.trim()) return
  emit('save', {
    name: form.name.trim(),
    deviceId: form.deviceId.trim(),
    description: form.description.trim(),
    status: form.status,
    templateId: form.templateMode === 'override' && form.templateId ? form.templateId : undefined,
  })
  open.value = false
}
</script>

<template>
  <BaseModal
    v-model="open"
    :title="device ? t('agentDevices.edit') : t('agentDevices.add')"
    :description="agent.name"
  >
    <form class="space-y-4" @submit.prevent="submit">
      <label class="block space-y-1.5">
        <span class="text-sm font-medium">{{ t('agents.name') }}</span>
        <input v-model="form.name" class="admin-input" required />
      </label>
      <label class="block space-y-1.5">
        <span class="text-sm font-medium">Device ID</span>
        <input v-model="form.deviceId" class="admin-input font-mono text-sm" required />
      </label>
      <label class="block space-y-1.5">
        <span class="text-sm font-medium">{{ t('devices.admission') }}</span>
        <select v-model="form.status" class="admin-input">
          <option value="online">{{ t('devices.enabled') }}</option>
          <option value="offline">{{ t('devices.disabled') }}</option>
        </select>
      </label>

      <fieldset class="space-y-2">
        <legend class="text-sm font-medium">{{ t('switcher.label') }}</legend>

        <label
          class="flex cursor-pointer items-start gap-2.5 rounded-lg border border-border/70 px-3 py-2.5"
          :class="form.templateMode === 'default' ? 'bg-secondary/60' : undefined"
        >
          <input v-model="form.templateMode" type="radio" value="default" class="mt-1 accent-foreground" />
          <span class="min-w-0">
            <span class="block text-sm font-medium">{{ t('agentDevices.default') }}</span>
            <span class="block truncate text-xs text-muted-foreground">
              {{ defaultTemplate?.name ?? t('common.notLinked') }}
            </span>
          </span>
        </label>

        <label
          class="flex cursor-pointer items-start gap-2.5 rounded-lg border border-border/70 px-3 py-2.5"
          :class="form.templateMode === 'override' ? 'bg-secondary/60' : undefined"
        >
          <input
            v-model="form.templateMode"
            type="radio"
            value="override"
            class="mt-1 accent-foreground"
            :disabled="templates.length === 0"
          />
          <span class="min-w-0 flex-1">
            <span class="block text-sm font-medium">{{ t('agentDevices.override') }}</span>
            <select
              v-model="form.templateId"
              class="admin-input mt-1.5"
              :disabled="form.templateMode !== 'override'"
              :aria-label="t('agentDevices.override')"
            >
              <option v-for="template in templates" :key="template.id" :value="template.id">
                {{ template.name }} · {{ template.language || '—' }}
              </option>
            </select>
          </span>
        </label>

        <p class="text-xs text-muted-foreground">{{ t('agentDetail.noTemplatesDescription') }}</p>
      </fieldset>

      <label class="block space-y-1.5">
        <span class="text-sm font-medium">{{ t('providers.descriptionField') }}</span>
        <textarea v-model="form.description" class="admin-textarea min-h-24" />
      </label>
      <div class="flex justify-end gap-2 pt-2">
        <Button type="button" variant="outline" @click="open = false">{{ t('common.cancel') }}</Button>
        <Button type="submit">{{ t('common.save') }}</Button>
      </div>
    </form>
  </BaseModal>
</template>

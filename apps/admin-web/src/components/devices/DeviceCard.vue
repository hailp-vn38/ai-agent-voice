<script setup lang="ts">
import { ArrowRight, Bot, CheckCircle2, Copy, Fingerprint, Layers3, Pencil, Settings2, ShieldOff, Trash2 } from '@lucide/vue'
import { computed, ref } from 'vue'

import VoiceDeviceIcon from '@/components/icons/VoiceDeviceIcon.vue'
import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { useI18n } from '@/composables/useI18n'
import type { AgentTemplate, Device } from '@/domain/admin'

const props = withDefaults(defineProps<{
  device: Device
  effectiveTemplate?: AgentTemplate
  agentName?: string
  showAgent?: boolean
  detailLink?: boolean
}>(), { showAgent: false, detailLink: false })

const emit = defineEmits<{
  edit: [device: Device]
  delete: [device: Device]
}>()

const { t } = useI18n()
const copied = ref(false)
const copyFailed = ref(false)

// The legacy DeviceStatus field is an admission setting, not WebSocket presence.
const admissionEnabled = computed(() => props.device.status === 'online')
const isOverride = computed(() => Boolean(props.device.templateId))

async function copyDeviceId() {
  copied.value = false
  copyFailed.value = false
  try {
    await navigator.clipboard.writeText(props.device.deviceId)
    copied.value = true
  } catch {
    copyFailed.value = true
  }
}
</script>

<template>
  <li class="dir-card device-card group" :data-enabled="admissionEnabled" data-device-card>
    <RouterLink v-if="detailLink" :to="{ name: 'device-detail', params: { deviceId: device.deviceId } }" :aria-label="t('deviceDetail.open', { name: device.name })" class="dir-card-hit" data-device-open />
    <div class="dir-card-head">
      <span class="device-icon" aria-hidden="true"><VoiceDeviceIcon /><i /></span>
      <div class="dir-card-title">
        <h3 :title="device.name">{{ device.name }}</h3>
        <p>{{ device.description || (device.kind === 'web_assistant' ? t('assistant.managedDevice') : t('deviceCard.type')) }}</p>
      </div>
      <div v-if="device.kind !== 'web_assistant'" class="dir-card-raise">
        <ActionMenu :label="t('common.actions')" panel-width="11rem">
          <MenuItem data-device-edit @select="emit('edit', device)">
            <Pencil class="size-4 shrink-0" aria-hidden="true" />
            {{ t('agentDevices.edit') }}
          </MenuItem>
          <div class="my-1 h-px bg-border" role="separator" />
          <MenuItem variant="danger" data-device-delete @select="emit('delete', device)">
            <Trash2 class="size-4 shrink-0" aria-hidden="true" />
            {{ t('agentDevices.delete') }}
          </MenuItem>
        </ActionMenu>
      </div>
    </div>

    <div class="device-status">
      <span class="dir-pill" :data-enabled="admissionEnabled" data-device-admission>
        <CheckCircle2 v-if="admissionEnabled" :size="13" aria-hidden="true" /><ShieldOff v-else :size="13" aria-hidden="true" />
        {{ admissionEnabled ? t('deviceCard.allowed') : t('deviceCard.blocked') }}
      </span>
      <span class="dir-tag">{{ t(device.kind === 'web_assistant' ? 'directoryUi.virtualDevice' : 'directoryUi.physicalDevice') }}</span>
      <RouterLink v-if="device.kind === 'web_assistant'" to="/system" class="device-managed dir-card-raise"><Settings2 :size="12" aria-hidden="true" />{{ t('nav.system') }}</RouterLink>
    </div>

    <div class="device-id">
      <Fingerprint :size="14" aria-hidden="true" />
      <code :title="t('deviceCard.deviceId')">{{ device.deviceId }}</code>
      <button type="button" class="device-copy dir-card-raise" :data-copied="copied" :aria-label="copied ? t('deviceCard.copied') : t('deviceCard.copyId')" data-device-copy @click="copyDeviceId">
        <CheckCircle2 v-if="copied" :size="14" aria-hidden="true" /><Copy v-else :size="14" aria-hidden="true" />
      </button>
    </div>
    <p v-if="copyFailed" role="alert" class="text-xs text-danger-foreground">{{ t('deviceCard.copyFailed') }}</p>

    <dl class="device-route">
      <div v-if="showAgent">
        <dt><Bot :size="12" aria-hidden="true" />{{ t('nav.agents') }}</dt>
        <dd>{{ agentName || device.agentId }}</dd>
      </div>
      <ArrowRight v-if="showAgent" :size="16" aria-hidden="true" />
      <div>
        <dt><Layers3 :size="12" aria-hidden="true" />{{ t('agentDevices.template') }}</dt>
        <dd>{{ effectiveTemplate?.name ?? t('common.notLinked') }}<span class="dir-tag">{{ isOverride ? t('agentDevices.override') : t('agentDevices.default') }}</span></dd>
      </div>
    </dl>
  </li>
</template>

<style scoped src="../admin/studio-directory-cards.css"></style>

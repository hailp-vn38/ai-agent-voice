<script setup lang="ts">
import AgentDeviceCard from '@/components/agents/AgentDeviceCard.vue'
import { useI18n } from '@/composables/useI18n'
import type { AgentTemplate, Device } from '@/domain/admin'

defineProps<{
  devices: Device[]
  effectiveTemplateById: (deviceId: string) => AgentTemplate | undefined
}>()

const emit = defineEmits<{
  edit: [device: Device]
  delete: [device: Device]
}>()

const { t } = useI18n()
</script>

<template>
  <ul v-if="devices.length" class="grid min-w-0 grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3" data-device-grid>
    <AgentDeviceCard
      v-for="device in devices"
      :key="device.id"
      :device="device"
      :effective-template="effectiveTemplateById(device.id)"
      @edit="emit('edit', $event)"
      @delete="emit('delete', $event)"
    />
  </ul>

  <div v-else class="rounded-xl border border-dashed border-border/80 bg-surface/30 px-4 py-12 text-center">
    <p class="text-sm font-medium">{{ t('agentDevices.empty') }}</p>
    <p class="mt-1 text-sm text-muted-foreground">{{ t('agentDevices.emptyDescription') }}</p>
  </div>
</template>

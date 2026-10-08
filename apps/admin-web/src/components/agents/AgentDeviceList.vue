<script setup lang="ts">
import { Plus } from '@lucide/vue'

import AgentDeviceCard from '@/components/agents/AgentDeviceCard.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { AgentTemplate, Device } from '@/domain/admin'

defineProps<{
  devices: Device[]
  effectiveTemplateById: (deviceId: string) => AgentTemplate | undefined
}>()

const emit = defineEmits<{
  add: []
  edit: [device: Device]
  delete: [device: Device]
}>()

const { t } = useI18n()
</script>

<template>
  <section class="min-w-0" aria-labelledby="agent-devices-heading">
    <header class="flex flex-wrap items-center justify-between gap-3">
      <div>
        <h2 id="agent-devices-heading" class="text-base font-semibold tracking-tight">
          {{ t('agentDevices.heading') }}
        </h2>
        <p class="mt-1 text-sm text-muted-foreground">
          {{ t('agentDevices.subtitle', { count: devices.length }) }}
        </p>
      </div>
      <Button size="sm" variant="outline" class="cursor-pointer" @click="emit('add')">
        <Plus class="size-4" aria-hidden="true" />
        {{ t('agentDevices.add') }}
      </Button>
    </header>

    <ul
      v-if="devices.length"
      class="mt-4 grid min-w-0 grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3"
      data-device-grid
    >
      <AgentDeviceCard
        v-for="device in devices"
        :key="device.id"
        :device="device"
        :effective-template="effectiveTemplateById(device.id)"
        @edit="emit('edit', $event)"
        @delete="emit('delete', $event)"
      />
    </ul>

    <div v-else class="mt-4 rounded-xl border border-dashed border-border/80 bg-surface/30 px-4 py-12 text-center">
      <p class="text-sm font-medium">{{ t('agentDevices.empty') }}</p>
      <p class="mt-1 text-sm text-muted-foreground">{{ t('agentDevices.emptyDescription') }}</p>
    </div>

    <p class="mt-4 text-xs text-muted-foreground">{{ t('devices.note') }}</p>
  </section>
</template>

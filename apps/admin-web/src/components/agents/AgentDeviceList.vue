<script setup lang="ts">
import { Plus } from '@lucide/vue'

import AgentDeviceRow from '@/components/agents/AgentDeviceRow.vue'
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
  <section
    class="rounded-xl border border-border/70 bg-card p-4 sm:p-5"
    aria-labelledby="agent-devices-heading"
  >
    <header class="flex flex-wrap items-center justify-between gap-3">
      <div>
        <h2 id="agent-devices-heading" class="text-base font-semibold tracking-tight">
          {{ t('agentDevices.heading') }}
        </h2>
        <p class="mt-1 text-sm text-muted-foreground">
          {{ t('agentDevices.subtitle', { count: devices.length }) }}
        </p>
      </div>
      <Button size="sm" variant="outline" @click="emit('add')">
        <Plus class="size-4" />
        {{ t('agentDevices.add') }}
      </Button>
    </header>

    <ul
      v-if="devices.length"
      class="mt-4 divide-y divide-border/60 overflow-hidden rounded-lg border border-border/70 bg-surface"
    >
      <AgentDeviceRow
        v-for="device in devices"
        :key="device.id"
        :device="device"
        :effective-template="effectiveTemplateById(device.id)"
        @edit="emit('edit', $event)"
        @delete="emit('delete', $event)"
      />
    </ul>

    <div v-else class="mt-4 rounded-lg border border-dashed border-border/80 px-4 py-12 text-center">
      <p class="text-sm font-medium">{{ t('agentDevices.empty') }}</p>
      <p class="mt-1 text-sm text-muted-foreground">{{ t('agentDevices.emptyDescription') }}</p>
    </div>
  </section>
</template>

<script setup lang="ts">
import DeviceCard from '@/components/devices/DeviceCard.vue'
import type { AgentTemplate, Device } from '@/domain/admin'

defineProps<{ device: Device; effectiveTemplate?: AgentTemplate }>()
defineEmits<{ edit: [device: Device]; delete: [device: Device] }>()
</script>

<template>
  <DeviceCard
    :device="device"
    :effective-template="effectiveTemplate"
    detail-link
    @edit="$emit('edit', $event)"
    @delete="$emit('delete', $event)"
  />
</template>

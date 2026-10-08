<script setup lang="ts">
import { computed } from 'vue'

import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { useI18n } from '@/composables/useI18n'
import type { AgentTemplate, Device } from '@/domain/admin'

const props = defineProps<{
  device: Device
  /** device.templateId override, else the agent default template. */
  effectiveTemplate?: AgentTemplate
}>()

const emit = defineEmits<{ edit: [device: Device]; delete: [device: Device] }>()

const { t, formatDateTime, formatRelative } = useI18n()

// The current Device read-model maps admission enabled onto the legacy online string.
const admissionEnabled = computed(() => props.device.status === 'online')
const isOverride = computed(() => props.device.templateId !== undefined)
const relativeLastSeen = computed(() => formatRelative(props.device.lastSeen))
const absoluteLastSeen = computed(() => formatDateTime(props.device.lastSeen))
</script>

<template>
  <li class="px-3.5 py-3">
    <div class="flex flex-wrap items-center gap-x-3 gap-y-1">
      <span
        class="size-2 shrink-0 rounded-full"
        :class="admissionEnabled ? 'bg-success' : 'bg-muted-foreground/40'"
        aria-hidden="true"
      />
      <p class="min-w-0 flex-1 truncate text-sm font-medium">{{ device.name }}</p>
      <p class="flex shrink-0 items-center gap-1.5 text-xs text-muted-foreground">
        <span>{{ admissionEnabled ? t('devices.enabled') : t('devices.disabled') }}</span>
        <template v-if="device.lastSeen">
          <span aria-hidden="true">·</span>
          <span :title="absoluteLastSeen">{{ relativeLastSeen }}</span>
        </template>
      </p>
      <ActionMenu class="shrink-0" :label="t('common.actions')" panel-width="11rem">
        <MenuItem @select="emit('edit', device)">{{ t('agentDevices.edit') }}</MenuItem>
        <MenuItem variant="danger" @select="emit('delete', device)">
          {{ t('agentDevices.delete') }}
        </MenuItem>
      </ActionMenu>
    </div>

    <p class="mt-1 pl-5 text-xs leading-relaxed text-muted-foreground">
      <span class="font-mono">{{ device.deviceId }}</span>
      <span v-if="device.description"> · {{ device.description }}</span>
    </p>

    <p class="mt-1.5 flex flex-wrap items-center gap-x-1.5 gap-y-1 pl-5 text-xs">
      <span class="text-muted-foreground">{{ t('agentDevices.template') }}</span>
      <span class="min-w-0 truncate font-medium text-foreground">
        {{ effectiveTemplate?.name ?? t('common.notLinked') }}
      </span>
      <span
        class="rounded-full px-1.5 py-0.5 text-[10px] font-medium"
        :class="isOverride ? 'bg-muted text-muted-foreground' : 'bg-secondary text-secondary-foreground'"
      >
        {{ isOverride ? t('agentDevices.override') : t('agentDevices.default') }}
      </span>
    </p>
  </li>
</template>

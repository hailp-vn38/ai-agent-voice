<script setup lang="ts">
import { ArrowUpRight, Bot, CheckCircle2, Copy, Fingerprint, Layers3, Pencil, ShieldOff } from '@lucide/vue'
import { computed, ref } from 'vue'

import VoiceDeviceIcon from '@/components/icons/VoiceDeviceIcon.vue'
import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { Button } from '@/components/ui/button'
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
  <li class="group relative flex min-w-0 flex-col rounded-xl border border-border/80 bg-card p-4 transition-colors hover:border-studio-cyan/35 hover:bg-surface-elevated/60" data-device-card>
    <RouterLink
      v-if="detailLink"
      :to="{ name: 'device-detail', params: { deviceId: device.deviceId } }"
      :aria-label="t('deviceDetail.open', { name: device.name })"
      class="absolute inset-0 z-10 cursor-pointer rounded-xl focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      data-device-open
    />
    <div class="pointer-events-none flex min-w-0 items-start gap-3">

      <span class="flex size-12 shrink-0 items-center justify-center rounded-xl border border-studio-cyan/20 bg-studio-cyan/10 text-studio-cyan">
        <VoiceDeviceIcon class="size-8" />
      </span>

      <div class="min-w-0 flex-1">
        <h3 class="truncate text-base font-semibold" :title="device.name">{{ device.name }}</h3>
        <p v-if="device.description" class="mt-1 line-clamp-2 break-words text-xs leading-5 text-muted-foreground">
          {{ device.description }}
        </p>
        <p v-else class="mt-1 text-xs text-muted-foreground">{{ t('deviceCard.type') }}</p>
      </div>

      <div class="pointer-events-auto relative z-20">
        <ActionMenu :label="t('common.actions')" panel-width="11rem">
        <MenuItem variant="danger" @select="emit('delete', device)">
          {{ t('agentDevices.delete') }}
        </MenuItem>
        </ActionMenu>
      </div>
    </div>

    <div class="pointer-events-none mt-3">
      <span
        class="inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-medium"
        :class="admissionEnabled ? 'bg-success/10 text-success-foreground' : 'bg-muted text-muted-foreground'"
        data-device-admission
      >
        <CheckCircle2 v-if="admissionEnabled" class="size-3.5" aria-hidden="true" />
        <ShieldOff v-else class="size-3.5" aria-hidden="true" />
        {{ admissionEnabled ? t('deviceCard.allowed') : t('deviceCard.blocked') }}
      </span>
    </div>

    <dl class="pointer-events-none mt-4 min-w-0 space-y-3 rounded-lg border border-border/60 bg-surface/60 px-3 py-3">
      <div class="flex min-w-0 items-center gap-2">
        <dt class="flex shrink-0 items-center gap-1.5 text-xs text-muted-foreground">
          <Fingerprint class="size-3.5" aria-hidden="true" />
          {{ t('deviceCard.deviceId') }}
        </dt>
        <dd class="pointer-events-auto relative z-20 ml-auto flex min-w-0 items-center gap-1.5">
          <code class="block min-w-0 truncate text-xs text-foreground" :title="device.deviceId">{{ device.deviceId }}</code>
          <button
            type="button"
            class="flex size-7 shrink-0 cursor-pointer items-center justify-center rounded-md text-muted-foreground transition hover:bg-accent hover:text-accent-foreground focus-visible:outline-2 focus-visible:outline-ring"
            :aria-label="copied ? t('deviceCard.copied') : t('deviceCard.copyId')"
            :title="copied ? t('deviceCard.copied') : t('deviceCard.copyId')"
            data-device-copy
            @click="copyDeviceId"
          >
            <CheckCircle2 v-if="copied" class="size-3.5" aria-hidden="true" />
            <Copy v-else class="size-3.5" aria-hidden="true" />
          </button>
        </dd>
      </div>
      <div v-if="showAgent" class="border-t border-border/60 pt-3">
        <dt class="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Bot class="size-3.5" aria-hidden="true" />{{ t('nav.agents') }}
        </dt>
        <dd class="mt-1.5 break-words text-sm font-medium">{{ agentName || device.agentId }}</dd>
      </div>
      <div class="border-t border-border/60 pt-3">
        <dt class="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Layers3 class="size-3.5" aria-hidden="true" />
          {{ t('agentDevices.template') }}
        </dt>
        <dd class="mt-1.5 flex min-w-0 flex-wrap items-center gap-2">
          <span class="min-w-0 break-words text-sm font-medium">{{ effectiveTemplate?.name ?? t('common.notLinked') }}</span>
          <span class="shrink-0 rounded-full bg-secondary px-2 py-0.5 text-[10px] font-medium text-secondary-foreground">
            {{ isOverride ? t('agentDevices.override') : t('agentDevices.default') }}
          </span>
        </dd>
      </div>
    </dl>

    <p v-if="copyFailed" role="alert" class="mt-2 text-xs text-danger-foreground">
      {{ t('deviceCard.copyFailed') }}
    </p>

    <div class="relative z-20 mt-auto flex items-center justify-between gap-2 pt-4">
      <span v-if="detailLink" class="pointer-events-none inline-flex items-center gap-1 text-xs font-medium text-studio-cyan">
        {{ t('deviceDetail.view') }} <ArrowUpRight class="size-3.5" aria-hidden="true" />
      </span>
      <Button variant="outline" size="sm" class="cursor-pointer" data-device-edit @click="emit('edit', device)">
        <Pencil class="size-3.5" aria-hidden="true" />
        {{ t('agentDevices.edit') }}
      </Button>
    </div>
  </li>
</template>

<script setup lang="ts">
import { Layers3, MonitorSmartphone, Pencil, Plus } from '@lucide/vue'

import AgentActionsMenu from '@/components/agents/AgentActionsMenu.vue'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import AgentVoiceIcon from '@/components/icons/AgentVoiceIcon.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { Agent } from '@/domain/admin'

defineProps<{
  agent: Agent
  templateCount: number
  deviceCount: number
}>()

const emit = defineEmits<{
  back: []
  addDevice: []
  editAgent: []
  deleteAgent: []
}>()

const { t } = useI18n()
</script>

<template>
  <DetailHeader :title="agent.name" :back-label="t('nav.agents')" data-agent-header @back="emit('back')">
    <template #icon>
      <span class="flex size-11 shrink-0 items-center justify-center rounded-xl border border-studio-violet/20 bg-studio-violet/10 text-studio-violet">
        <AgentVoiceIcon class="size-7" />
      </span>
    </template>
    <template #details>
      <div class="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
        <p class="max-w-80 min-w-0 truncate" :title="agent.description || t('agentHeader.type')">
          {{ agent.description || t('agentHeader.type') }}
        </p>
        <dl class="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1" data-agent-metadata>
          <div class="inline-flex items-center gap-1">
            <dt class="sr-only">{{ t('switcher.heading') }}</dt>
            <Layers3 class="size-3.5 text-studio-violet" aria-hidden="true" />
            <dd>{{ t('count.templates', { count: templateCount }) }}</dd>
          </div>
          <div class="inline-flex items-center gap-1">
            <dt class="sr-only">{{ t('agentDevices.heading') }}</dt>
            <MonitorSmartphone class="size-3.5 text-studio-cyan" aria-hidden="true" />
            <dd>{{ t('count.devices', { count: deviceCount }) }}</dd>
          </div>
        </dl>
      </div>
    </template>
    <template #actions>
      <div data-agent-actions class="contents">
        <Button size="sm" variant="outline" class="cursor-pointer" data-agent-edit @click="emit('editAgent')">
          <Pencil class="size-4" aria-hidden="true" />
          {{ t('agents.edit') }}
        </Button>
        <Button size="sm" class="cursor-pointer" data-agent-add-device @click="emit('addDevice')">
          <Plus class="size-4" aria-hidden="true" />
          {{ t('agentDevices.add') }}
        </Button>
        <AgentActionsMenu @delete-agent="emit('deleteAgent')" />
      </div>
    </template>
  </DetailHeader>
</template>

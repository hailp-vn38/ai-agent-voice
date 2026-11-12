<script setup lang="ts">
import { Layers3, MonitorSmartphone, Pencil, Plus } from '@lucide/vue'

import AgentActionsMenu from '@/components/agents/AgentActionsMenu.vue'
import DetailHero from '@/components/admin/DetailHero.vue'
import AgentVoiceIcon from '@/components/icons/AgentVoiceIcon.vue'
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
  <DetailHero tone="agents" :eyebrow="t('directoryUi.agentIdentity')" :title="agent.name" :description="agent.description || t('agentHeader.type')" :back-label="t('nav.agents')" data-agent-header @back="emit('back')">
    <template #icon><AgentVoiceIcon class="size-9" /></template>
    <template #actions>
      <div data-agent-actions class="contents">
        <button type="button" class="hero-primary" data-agent-add-device @click="emit('addDevice')">
          <Plus :size="16" aria-hidden="true" />
          {{ t('agentDevices.add') }}
        </button>
        <button type="button" class="hero-ghost" data-agent-edit @click="emit('editAgent')">
          <Pencil :size="16" aria-hidden="true" />
          {{ t('agents.edit') }}
        </button>
        <span class="hero-menu"><AgentActionsMenu @delete-agent="emit('deleteAgent')" /></span>
      </div>
    </template>
    <template #facts>
      <div class="contents" data-agent-metadata>
        <div class="hero-fact"><span><Layers3 :size="13" aria-hidden="true" />{{ t('switcher.heading') }}</span><strong>{{ t('count.templates', { count: templateCount }) }}</strong></div>
        <div class="hero-fact"><span><MonitorSmartphone :size="13" aria-hidden="true" />{{ t('agentDevices.heading') }}</span><strong>{{ t('count.devices', { count: deviceCount }) }}</strong></div>
      </div>
    </template>
  </DetailHero>
</template>

<script setup lang="ts">
import { ArrowLeft, Layers3, MonitorSmartphone, Pencil, Plus } from '@lucide/vue'

import AgentActionsMenu from '@/components/agents/AgentActionsMenu.vue'
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
  <header class="studio-panel min-w-0 p-4 sm:p-5" data-agent-header>
    <Button
      size="sm"
      variant="ghost"
      class="-ml-2 cursor-pointer text-muted-foreground hover:text-foreground"
      data-agent-back
      @click="emit('back')"
    >
      <ArrowLeft class="size-4" aria-hidden="true" />
      {{ t('nav.agents') }}
    </Button>

    <div class="mt-3 flex min-w-0 flex-col gap-4 lg:flex-row lg:items-center lg:justify-between">
      <div class="flex min-w-0 items-center gap-3.5">
        <span class="flex size-14 shrink-0 items-center justify-center rounded-2xl border border-studio-violet/20 bg-studio-violet/10 text-studio-violet">
          <AgentVoiceIcon class="size-9" />
        </span>
        <div class="min-w-0">
          <h1 class="break-words text-2xl font-semibold tracking-tight sm:text-3xl">{{ agent.name }}</h1>
          <p class="mt-1 line-clamp-2 max-w-2xl break-words text-sm text-muted-foreground">
            {{ agent.description || t('agentHeader.type') }}
          </p>
        </div>
      </div>

      <div class="flex shrink-0 flex-wrap items-center gap-2" data-agent-actions>
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
    </div>

    <dl class="mt-4 flex flex-wrap items-center gap-x-5 gap-y-2 border-t border-border/70 pt-3 text-xs text-muted-foreground">
      <div class="inline-flex items-center gap-1.5">
        <dt class="sr-only">{{ t('switcher.heading') }}</dt>
        <Layers3 class="size-4 text-studio-violet" aria-hidden="true" />
        <dd>{{ t('count.templates', { count: templateCount }) }}</dd>
      </div>
      <div class="inline-flex items-center gap-1.5">
        <dt class="sr-only">{{ t('agentDevices.heading') }}</dt>
        <MonitorSmartphone class="size-4 text-studio-cyan" aria-hidden="true" />
        <dd>{{ t('count.devices', { count: deviceCount }) }}</dd>
      </div>
    </dl>
  </header>
</template>

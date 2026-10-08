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
  <header class="studio-panel min-w-0 px-3 py-3 sm:px-4 sm:py-3.5" data-agent-header>
    <div class="flex min-w-0 flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
      <div class="flex min-w-0 items-center gap-2.5 sm:gap-3">
        <Button
          size="sm"
          variant="ghost"
          class="h-9 shrink-0 cursor-pointer px-2 text-muted-foreground hover:text-foreground sm:px-2.5"
          :aria-label="t('nav.agents')"
          data-agent-back
          @click="emit('back')"
        >
          <ArrowLeft class="size-4" aria-hidden="true" />
          <span class="hidden sm:inline">{{ t('nav.agents') }}</span>
        </Button>

        <span class="h-9 w-px shrink-0 bg-border/80" aria-hidden="true" />

        <span
          class="flex size-11 shrink-0 items-center justify-center rounded-xl border border-studio-violet/20 bg-studio-violet/10 text-studio-violet"
        >
          <AgentVoiceIcon class="size-7" />
        </span>

        <div class="min-w-0 flex-1">
          <h1 class="line-clamp-2 break-words text-xl font-semibold leading-tight tracking-tight sm:text-2xl" :title="agent.name">
            {{ agent.name }}
          </h1>
          <div class="mt-1 flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
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
        </div>
      </div>

      <div class="flex shrink-0 flex-wrap items-center gap-2 pl-0 lg:pl-2" data-agent-actions>
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
  </header>
</template>

<script setup lang="ts">
import { ArrowLeft, Layers, MonitorSmartphone, Pencil, Plus } from '@lucide/vue'

import AgentActionsMenu from '@/components/agents/AgentActionsMenu.vue'
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
  <header class="rounded-xl border border-border/70 bg-card p-4 sm:p-5">
    <Button variant="ghost" size="sm" class="-ml-2" @click="emit('back')">
      <ArrowLeft class="size-4" />
      {{ t('nav.agents') }}
    </Button>

    <div class="mt-3 flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
      <div class="min-w-0">
        <h1 class="text-3xl font-semibold tracking-tight">{{ agent.name }}</h1>
        <p v-if="agent.description" class="mt-1.5 max-w-2xl text-sm leading-relaxed text-muted-foreground">
          {{ agent.description }}
        </p>

        <dl class="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
          <div class="inline-flex items-center gap-1.5">
            <dt class="sr-only">{{ t('switcher.heading') }}</dt>
            <Layers class="size-3.5" aria-hidden="true" />
            <dd>{{ t('count.templates', { count: templateCount }) }}</dd>
          </div>
          <div class="inline-flex items-center gap-1.5">
            <dt class="sr-only">{{ t('agentDevices.heading') }}</dt>
            <MonitorSmartphone class="size-3.5" aria-hidden="true" />
            <dd>{{ t('count.devices', { count: deviceCount }) }}</dd>
          </div>
        </dl>
      </div>

      <div class="flex shrink-0 flex-wrap items-center gap-2">
        <Button size="sm" variant="outline" @click="emit('addDevice')">
          <Plus class="size-4" />
          {{ t('agentDevices.add') }}
        </Button>
        <Button size="sm" variant="outline" @click="emit('editAgent')">
          <Pencil class="size-4" />
          {{ t('agents.edit') }}
        </Button>
        <AgentActionsMenu @deleteAgent="emit('deleteAgent')" />
      </div>
    </div>
  </header>
</template>

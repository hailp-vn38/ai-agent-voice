<script setup lang="ts">
import { Bot, Layers, MonitorSmartphone, Plus, ArrowUpRight } from '@lucide/vue'

import ProviderChip from '@/components/admin/ProviderChip.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { Agent, AgentTemplate, ProviderInstance } from '@/domain/admin'

defineProps<{
  agent: Agent
  defaultTemplate?: AgentTemplate
  defaultTemplateProviders: ProviderInstance[]
  templateCount: number
  deviceCount: number
}>()
const emit = defineEmits<{ open: []; addDevice: [] }>()
const { t } = useI18n()
</script>

<template>
  <article class="studio-panel group p-5 transition-all hover:-translate-y-0.5 hover:border-studio-violet/40 hover:shadow-lg">
    <button type="button" class="flex w-full items-start gap-3 rounded-lg text-left focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring" :aria-label="agent.name" @click="emit('open')">
      <span class="flex size-12 shrink-0 items-center justify-center rounded-xl bg-studio-violet/10 text-studio-violet">
        <Bot class="size-6" aria-hidden="true" />
      </span>
      <span class="min-w-0 flex-1">
        <span class="block truncate text-lg font-semibold">{{ agent.name }}</span>
        <span class="mt-1 block min-h-10 text-sm text-muted-foreground">{{ agent.description || '—' }}</span>
      </span>
      <ArrowUpRight class="size-4 shrink-0 text-muted-foreground transition group-hover:text-studio-violet" aria-hidden="true" />
    </button>

    <div class="mt-5 flex flex-wrap items-center gap-4 text-xs text-muted-foreground">
      <span class="inline-flex items-center gap-1.5">
        <Layers class="size-4" aria-hidden="true" />
        {{ t('count.templates', { count: templateCount }) }}
      </span>
      <span class="inline-flex items-center gap-1.5">
        <MonitorSmartphone class="size-4" aria-hidden="true" />
        {{ t('count.devices', { count: deviceCount }) }}
      </span>
    </div>

    <div class="mt-4 border-t border-border/70 pt-4">
      <p v-if="defaultTemplate" class="mb-3 truncate text-xs font-medium text-muted-foreground">
        {{ t('agents.defaultTemplate', { name: defaultTemplate.name, language: defaultTemplate.language || '—' }) }}
      </p>
      <p v-else class="mb-3 text-xs text-muted-foreground">{{ t('agents.noTemplate') }}</p>
      <div class="flex min-h-7 flex-wrap gap-1.5">
        <ProviderChip v-for="provider in defaultTemplateProviders" :key="provider.id" :provider="provider" />
        <span v-if="defaultTemplate && defaultTemplateProviders.length === 0" class="text-xs text-muted-foreground">
          {{ t('studio.pipeline.default') }}
        </span>
      </div>
    </div>

    <div class="mt-5 flex justify-end">
      <Button size="sm" variant="outline" @click.stop="emit('addDevice')">
        <Plus class="size-4" aria-hidden="true" />
        {{ t('agentDevices.add') }}
      </Button>
    </div>
  </article>
</template>

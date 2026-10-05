<script setup lang="ts">
import { Layers, MonitorSmartphone, Plus } from '@lucide/vue'

import ProviderChip from '@/components/admin/ProviderChip.vue'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { useI18n } from '@/composables/useI18n'
import type { Agent, AgentTemplate, ProviderInstance } from '@/domain/admin'

defineProps<{
  agent: Agent
  defaultTemplate?: AgentTemplate
  defaultTemplateProviders: ProviderInstance[]
  templateCount: number
  deviceCount: number
}>()

const emit = defineEmits<{
  open: []
  addDevice: []
}>()

const { t } = useI18n()
</script>

<template>
  <Card class="group cursor-pointer transition hover:-translate-y-0.5 hover:shadow-md" @click="emit('open')">
    <CardHeader>
      <div class="flex items-start justify-between gap-4">
        <div class="min-w-0">
          <CardTitle class="truncate text-lg">{{ agent.name }}</CardTitle>
          <CardDescription class="mt-1 line-clamp-2 min-h-10">{{ agent.description }}</CardDescription>
        </div>
        <Button size="sm" variant="outline" @click.stop="emit('addDevice')">
          <Plus class="size-4" />
          {{ t('agentDevices.add') }}
        </Button>
      </div>
    </CardHeader>
    <CardContent class="space-y-4">
      <div class="flex flex-wrap items-center gap-x-4 gap-y-2 text-sm text-muted-foreground">
        <span class="inline-flex items-center gap-1.5">
          <Layers class="size-4" />
          {{ t('count.templates', { count: templateCount }) }}
        </span>
        <span class="inline-flex items-center gap-1.5">
          <MonitorSmartphone class="size-4" />
          {{ t('count.devices', { count: deviceCount }) }}
        </span>
      </div>

      <div v-if="defaultTemplate" class="space-y-2">
        <p class="text-xs text-muted-foreground">
          {{
            t('agents.defaultTemplate', {
              name: defaultTemplate.name,
              language: defaultTemplate.language || '—',
            })
          }}
        </p>
        <div class="flex flex-wrap gap-2">
          <ProviderChip
            v-for="provider in defaultTemplateProviders"
            :key="provider.id"
            :provider="provider"
          />
          <span v-if="defaultTemplateProviders.length === 0" class="text-xs text-muted-foreground">
            {{ t('agents.noProviders') }}
          </span>
        </div>
      </div>
      <p v-else class="text-xs text-muted-foreground">{{ t('agents.noTemplate') }}</p>
    </CardContent>
  </Card>
</template>

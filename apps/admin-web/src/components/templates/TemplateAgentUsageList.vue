<script setup lang="ts">
import { Link2Off } from '@lucide/vue'

import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { Agent } from '@/domain/admin'

const props = defineProps<{
  agents: Agent[]
  deviceCountByAgent: (agentId: string) => number
  /** `agent.defaultTemplateId === template.id` marks the Default row. */
  isDefault: (agentId: string) => boolean
}>()

const { t } = useI18n()

const emit = defineEmits<{
  viewAgent: [agentId: string]
  unlink: [agentId: string]
}>()

function deviceCount(agent: Agent) {
  return props.deviceCountByAgent(agent.id)
}
</script>

<template>
  <section
    class="rounded-xl border border-border/70 bg-card p-4 sm:p-5"
    aria-labelledby="template-agents-heading"
  >
    <header class="flex flex-wrap items-baseline justify-between gap-3">
      <h2 id="template-agents-heading" class="text-base font-semibold tracking-tight">
        {{ t('templateUsage.heading') }}
      </h2>
      <p class="text-sm text-muted-foreground">{{ agents.length }}</p>
    </header>

    <ul v-if="agents.length" class="mt-4 divide-y divide-border/60 overflow-hidden rounded-lg border border-border/70 bg-surface">
      <li v-for="agent in agents" :key="agent.id" class="flex flex-wrap items-center gap-x-3 gap-y-2 px-3.5 py-3">
        <div class="min-w-0 flex-1">
          <div class="flex flex-wrap items-center gap-2">
            <p class="min-w-0 truncate text-sm font-medium">{{ agent.name }}</p>
            <Badge variant="secondary" class="px-1.5 py-0 text-[10px]">
              {{ isDefault(agent.id) ? t('templateUsage.default') : t('templateUsage.linked') }}
            </Badge>
          </div>
          <p class="mt-0.5 text-xs text-muted-foreground">
            {{ isDefault(agent.id) ? t('templateUsage.defaultTemplate') : t('templateUsage.linkedTemplate') }}
            · {{ t('count.devices', { count: deviceCount(agent) }) }}
          </p>
        </div>

        <div class="flex shrink-0 items-center gap-2">
          <Button size="sm" variant="outline" @click="emit('viewAgent', agent.id)">
            {{ t('templateUsage.viewAgent') }}
          </Button>
          <ActionMenu :label="t('templatesActions.label', { name: agent.name })" panel-width="15rem">
            <MenuItem variant="danger" @select="emit('unlink', agent.id)">
              <Link2Off class="size-4 shrink-0" aria-hidden="true" />
              {{ t('templateUsage.unlink') }}
            </MenuItem>
          </ActionMenu>
        </div>
      </li>
    </ul>

    <div v-else class="mt-4 rounded-lg border border-dashed border-border/80 px-4 py-12 text-center">
      <p class="text-sm font-medium">{{ t('templateUsage.unused') }}</p>
      <p class="mt-1 text-sm text-muted-foreground">{{ t('templateUsage.unusedDescription') }}</p>
    </div>
  </section>
</template>

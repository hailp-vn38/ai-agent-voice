<script setup lang="ts">
import { ArrowUpRight, Link2, Link2Off, Users } from '@lucide/vue'

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
  linkToAgent: []
}>()

function deviceCount(agent: Agent) {
  return props.deviceCountByAgent(agent.id)
}
</script>

<template>
  <section
    class="template-agent-usage"
    aria-labelledby="template-agents-heading"
  >
    <header class="usage-heading">
      <div class="usage-heading-copy"><span class="usage-icon" aria-hidden="true"><Users :size="18" /></span><h2 id="template-agents-heading">{{ t('templateUsage.heading') }}</h2><span class="usage-count">{{ agents.length }}</span></div>
      <Button size="sm" variant="outline" @click="emit('linkToAgent')"><Link2 :size="14" aria-hidden="true" />{{ t('templatesActions.linkToAgent') }}</Button>
    </header>

    <ul v-if="agents.length" class="usage-agent-grid">
      <li v-for="agent in agents" :key="agent.id" class="usage-agent-card">
        <span class="usage-agent-avatar" aria-hidden="true">{{ agent.name.slice(0, 1).toUpperCase() }}</span>
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
            <ArrowUpRight :size="13" aria-hidden="true" />
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

    <div v-else class="usage-empty">
      <Users :size="30" aria-hidden="true" />
      <p class="text-sm font-medium">{{ t('templateUsage.unused') }}</p>
      <p class="mt-1 text-sm text-muted-foreground">{{ t('templateUsage.unusedDescription') }}</p>
    </div>
  </section>
</template>

<style scoped>
.template-agent-usage { min-width: 0; padding: 24px; border: 1px solid var(--border); border-radius: 22px; background: radial-gradient(ellipse at 100% 0, color-mix(in oklab, var(--studio-violet) 4%, transparent), transparent 60%), var(--card); }
.usage-heading { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 12px; }
.usage-heading-copy { display: flex; align-items: center; gap: 11px; }
.usage-heading h2 { font-size: 15px; font-weight: 600; letter-spacing: -.02em; }
.usage-icon { display: grid; place-items: center; width: 34px; height: 34px; border-radius: 11px; background: color-mix(in oklab, var(--studio-violet) 8%, transparent); color: var(--studio-violet); }
.usage-count { padding: 3px 7px; border: 1px solid var(--border); border-radius: 7px; font: 11px ui-monospace, monospace; color: var(--muted-foreground); }
.usage-agent-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; margin-top: 18px; }
.usage-agent-card { display: flex; flex-wrap: wrap; align-items: center; gap: 12px; min-width: 0; padding: 16px; border: 1px solid var(--border); border-radius: 15px; background: var(--background); transition: border-color .2s; }
.usage-agent-card:hover { border-color: color-mix(in oklab, var(--studio-violet) 35%, var(--border)); }
.usage-agent-avatar { display: grid; place-items: center; width: 40px; height: 40px; flex-shrink: 0; border-radius: 13px; font-size: 17px; font-weight: 600; color: var(--studio-violet); background: color-mix(in oklab, var(--studio-violet) 10%, transparent); }
.usage-empty { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 7px; padding: 30px 16px; margin-top: 18px; border: 1px dashed var(--border); border-radius: 15px; background: var(--background); }
.usage-empty > svg { margin-bottom: 6px; color: var(--studio-violet); opacity: .6; }
@media (max-width: 850px) { .usage-agent-grid { grid-template-columns: minmax(0, 1fr); } }
@media (max-width: 560px) { .template-agent-usage { padding: 16px; border-radius: 18px; } }
@media (prefers-reduced-motion: reduce) { .usage-agent-card { transition: none; } }
</style>

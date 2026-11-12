<script setup lang="ts">
import { Copy, Eye, Link2, Link2Off, Pencil, Trash2 } from '@lucide/vue'
import { computed } from 'vue'

import TemplatePromptCard from '@/components/templates/TemplatePromptCard.vue'
import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { Badge } from '@/components/ui/badge'
import { useI18n } from '@/composables/useI18n'
import type { AgentTemplate } from '@/domain/admin'

const props = withDefaults(defineProps<{
  template: AgentTemplate
  providerCount: number
  agentCount: number
  deviceCount: number
  context?: 'agent' | 'template'
}>(), { context: 'agent' })

const emit = defineEmits<{
  viewTemplate: []
  editTemplate: []
  linkToAgent: []
  copyTemplate: []
  unlinkFromAgent: []
  deleteTemplate: []
  savePrompt: [prompt: string]
}>()

const { t, formatRelative } = useI18n()

/** A shared template cannot be deleted until every agent unlinks it. */
const shared = computed(() => props.agentCount > 0)
const relativeUpdatedAt = computed(() => formatRelative(props.template.updatedAt))
</script>

<template>
  <section
    class="template-config-panel"
    aria-labelledby="template-config-heading"
  >
    <header class="config-heading flex items-start justify-between gap-3">
      <div class="min-w-0">
        <h2 id="template-config-heading" class="text-base font-semibold tracking-tight">
          {{ t('templateConfig.heading') }}
        </h2>
        <p class="mt-1 text-sm text-muted-foreground">{{ t('templateConfig.description') }}</p>
      </div>

      <ActionMenu :label="t('templatesActions.label', { name: template.name })" panel-width="15rem">
        <MenuItem v-if="context === 'agent'" @select="emit('viewTemplate')">
          <Eye class="size-4 shrink-0" aria-hidden="true" />
          {{ t('templateConfig.viewTemplate') }}
        </MenuItem>
        <MenuItem @select="emit('editTemplate')">
          <Pencil class="size-4 shrink-0" aria-hidden="true" />
          {{ t('templateConfig.editTemplate') }}
        </MenuItem>
        <MenuItem @select="emit('linkToAgent')">
          <Link2 class="size-4 shrink-0" aria-hidden="true" />
          {{ t('templateConfig.linkToAgent') }}
        </MenuItem>
        <MenuItem @select="emit('copyTemplate')">
          <Copy class="size-4 shrink-0" aria-hidden="true" />
          {{ t('templateConfig.copyTemplate') }}
        </MenuItem>
        <MenuItem v-if="context === 'agent'" variant="danger" @select="emit('unlinkFromAgent')">
          <Link2Off class="size-4 shrink-0" aria-hidden="true" />
          {{ t('templateConfig.unlinkFromAgent') }}
        </MenuItem>
        <div class="my-1 h-px bg-border" role="separator" />
        <MenuItem :disabled="shared" variant="danger" @select="emit('deleteTemplate')">
          <Trash2 class="size-4 shrink-0" aria-hidden="true" />
          {{ t('templateConfig.delete') }}
        </MenuItem>
      </ActionMenu>
    </header>

    <div class="config-identity rounded-xl border border-border/70 bg-surface px-4 py-3">
      <div class="flex flex-wrap items-center gap-2">
        <p class="text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
          {{ t('templateConfig.template') }}
        </p>
        <Badge v-if="shared" variant="secondary">{{ t('templateConfig.shared') }}</Badge>
      </div>
      <p class="mt-1.5 break-words text-sm font-semibold">{{ template.name }}</p>
      <p v-if="template.description" class="mt-1 text-xs leading-relaxed text-muted-foreground">
        {{ template.description }}
      </p>
    </div>

    <div class="config-language rounded-xl border border-border/70 bg-surface px-4 py-3">
      <p class="text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        {{ t('templateConfig.language') }}
      </p>
      <p class="mt-1.5 break-words text-sm font-semibold">{{ template.language || '—' }}</p>
    </div>

    <div class="config-prompt">
      <TemplatePromptCard :prompt="template.prompt" @save="emit('savePrompt', $event)" />
    </div>

    <div class="config-usage">
      <h3 class="text-sm font-semibold">{{ t('templateConfig.usage') }}</h3>
      <dl class="mt-3 divide-y divide-border/60 rounded-lg border border-border/70 bg-surface px-3.5">
        <div class="flex items-center justify-between gap-3 py-2.5">
          <dt class="text-xs text-muted-foreground">{{ t('templateConfig.providers') }}</dt>
          <dd class="truncate text-sm font-medium">{{ providerCount }}</dd>
        </div>
        <div class="flex items-center justify-between gap-3 py-2.5">
          <dt class="text-xs text-muted-foreground">{{ t('templateConfig.agentsUsing') }}</dt>
          <dd class="truncate text-sm font-medium">{{ agentCount }}</dd>
        </div>
        <div class="flex items-center justify-between gap-3 py-2.5">
          <dt class="text-xs text-muted-foreground">{{ t('templateConfig.devicesResolving') }}</dt>
          <dd class="truncate text-sm font-medium">{{ deviceCount }}</dd>
        </div>
        <div class="flex items-center justify-between gap-3 py-2.5">
          <dt class="text-xs text-muted-foreground">{{ t('templateConfig.updated') }}</dt>
          <dd class="truncate text-sm font-medium">{{ relativeUpdatedAt }}</dd>
        </div>
        <div class="flex items-center justify-between gap-3 py-2.5">
          <dt class="text-xs text-muted-foreground">{{ t('templateConfig.templateId') }}</dt>
          <dd class="truncate font-mono text-xs text-muted-foreground" :title="template.id">
            {{ template.id }}
          </dd>
        </div>
      </dl>
    </div>
  </section>
</template>

<style scoped>
.template-config-panel { display: grid; grid-template-columns: minmax(0, 1.5fr) minmax(260px, 1fr); grid-template-areas: 'heading heading' 'prompt identity' 'prompt language' 'usage usage'; gap: 14px 22px; padding: 24px; border: 1px solid var(--border); border-radius: 22px; background: var(--card); }
.config-heading { grid-area: heading; padding-bottom: 6px; }
.config-identity { grid-area: identity; background: color-mix(in oklab, var(--studio-violet) 4%, var(--background)); }
.config-language { grid-area: language; }
.config-prompt { grid-area: prompt; min-width: 0; padding: 20px; border: 1px solid var(--border); border-radius: 16px; background: radial-gradient(ellipse at 0 0, color-mix(in oklab, var(--studio-violet) 4%, transparent), transparent 70%), var(--background); }
.config-prompt :deep(.max-h-60) { max-height: 340px; border-color: transparent; background: transparent; padding: 12px 0; font-size: 14px; line-height: 1.9; }
.config-usage { grid-area: usage; min-width: 0; }
.config-usage dl { display: grid; grid-template-columns: repeat(5, minmax(0, 1fr)); margin-top: 10px; padding: 0; border-radius: 12px; }
.config-usage dl > div { display: block; min-width: 0; padding: 12px 16px; border-top: 0; }
.config-usage dl > div + div { border-left: 1px solid var(--border); }
.config-usage dd { margin-top: 6px; }
@media (max-width: 760px) {
  .template-config-panel { grid-template-columns: minmax(0, 1fr); grid-template-areas: 'heading' 'identity' 'language' 'prompt' 'usage'; padding: 16px; gap: 14px; border-radius: 18px; }
  .config-prompt { padding: 16px; }
  .config-usage dl { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .config-usage dl > div { padding: 10px; }
  .config-usage dl > div + div { border-left: 0; }
}
</style>

<script setup lang="ts">
import { Copy, Eye, Link2, Link2Off, Pencil, Trash2 } from '@lucide/vue'
import { computed } from 'vue'

import TemplatePromptCard from '@/components/templates/TemplatePromptCard.vue'
import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { Badge } from '@/components/ui/badge'
import { useI18n } from '@/composables/useI18n'
import type { AgentTemplate } from '@/domain/admin'

const props = defineProps<{
  template: AgentTemplate
  providerCount: number
  agentCount: number
  deviceCount: number
}>()

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
    class="rounded-xl border border-border/70 bg-card p-4 sm:p-5"
    aria-labelledby="template-config-heading"
  >
    <header class="flex items-start justify-between gap-3">
      <div class="min-w-0">
        <h2 id="template-config-heading" class="text-base font-semibold tracking-tight">
          {{ t('templateConfig.heading') }}
        </h2>
        <p class="mt-1 text-sm text-muted-foreground">{{ t('templateConfig.description') }}</p>
      </div>

      <ActionMenu :label="t('templatesActions.label', { name: template.name })" panel-width="15rem">
        <MenuItem @select="emit('viewTemplate')">
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
        <MenuItem variant="danger" @select="emit('unlinkFromAgent')">
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

    <div class="mt-5 rounded-lg border border-border/70 bg-surface px-3.5 py-3">
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

    <div class="mt-4 rounded-lg border border-border/70 bg-surface px-3.5 py-3">
      <p class="text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        {{ t('templateConfig.language') }}
      </p>
      <p class="mt-1.5 break-words text-sm font-semibold">{{ template.language || '—' }}</p>
    </div>

    <div class="mt-6">
      <TemplatePromptCard :prompt="template.prompt" @save="emit('savePrompt', $event)" />
    </div>

    <div class="mt-6">
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

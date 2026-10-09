<script setup lang="ts">
import { computed } from 'vue'

import TemplateActionsMenu from '@/components/templates/TemplateActionsMenu.vue'
import TemplatePipelineSummary from '@/components/templates/TemplatePipelineSummary.vue'
import { Badge } from '@/components/ui/badge'
import { useI18n } from '@/composables/useI18n'
import type { AgentTemplate } from '@/domain/admin'

const { t } = useI18n()

const AGENT_PREVIEW_LIMIT = 3

const props = defineProps<{
  template: AgentTemplate
  agentCount: number
  /** Names of the agents using this template, for a compact preview. */
  agentNames: string[]
  providerNameById: (providerId?: string) => string
}>()

const emit = defineEmits<{
  open: []
  edit: []
  viewDetails: []
  linkToAgent: []
  copy: []
  delete: []
}>()

const previewedAgentNames = computed(() => props.agentNames.slice(0, AGENT_PREVIEW_LIMIT))
const overflowCount = computed(() => Math.max(0, props.agentNames.length - AGENT_PREVIEW_LIMIT))
const used = computed(() => props.agentCount > 0)
</script>

<template>
  <article
    class="flex cursor-pointer flex-col gap-3 rounded-xl border border-border/70 bg-card p-4 transition-colors hover:border-ring/50 hover:bg-accent/40"
    @click="emit('open')"
  >
    <div class="flex items-start justify-between gap-3">
      <div class="min-w-0">
        <h3 class="truncate text-base font-semibold tracking-tight">
          {{ template.name }}
        </h3>
        <p v-if="template.description" class="mt-1 line-clamp-2 text-sm text-muted-foreground">
          {{ template.description }}
        </p>
        <p v-else class="mt-1 text-sm text-muted-foreground">{{ t('common.noDescription') }}</p>
      </div>

      <div class="flex shrink-0 items-center gap-2" @click.stop>
        <TemplateActionsMenu
          :template="template"
          show-edit
          @edit="emit('edit')"
          @view-details="emit('open')"
          @link-to-agent="emit('linkToAgent')"
          @copy="emit('copy')"
          @delete="emit('delete')"
        />
      </div>
    </div>

    <div class="flex flex-wrap items-center gap-2">
      <Badge variant="secondary">{{ template.language || t('templateCard.noLanguage') }}</Badge>
    </div>

    <TemplatePipelineSummary :template="template" :provider-name-by-id="providerNameById" />

    <div class="mt-auto flex flex-wrap items-center gap-x-2 gap-y-1 border-t border-border/60 pt-3 text-xs">
      <span class="font-medium text-foreground">
        {{ t('count.agents', { count: agentCount }) }}
      </span>
      <span v-if="used && previewedAgentNames.length" class="min-w-0 truncate text-muted-foreground">
        {{ previewedAgentNames.join(', ') }}<template v-if="overflowCount > 0">, {{ t('count.more', { count: overflowCount }) }}</template>
      </span>
      <span v-else-if="used" class="text-muted-foreground">{{ t('templateCard.linked') }}</span>
      <span v-else class="text-muted-foreground">{{ t('templateCard.unused') }}</span>
    </div>
  </article>
</template>

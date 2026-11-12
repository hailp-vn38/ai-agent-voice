<script setup lang="ts">
import { computed } from 'vue'

import { useI18n } from '@/composables/useI18n'
import { providerTypes, type AgentTemplate } from '@/domain/admin'
import { providerTypeIcons } from '@/lib/providerTypeIcons'

const props = defineProps<{
  template: AgentTemplate
  providerNameById: (providerId?: string) => string
}>()

/** Voice path in pipeline order. Vision is an LLM capability, so it branches below. */
const { t, providerTypeLabel } = useI18n()

const chain = computed(() =>
  providerTypes
    .map((type) => ({
      type,
      label: providerTypeLabel(type),
      name: props.template.providerBindings[type] ? props.providerNameById(props.template.providerBindings[type]) : '',
    })),
)
</script>

<template>
  <ol class="template-mini-pipeline" :aria-label="t('templateForm.providers')">
    <li v-for="step in chain" :key="step.type" :data-bound="Boolean(step.name)">
      <span class="pipeline-node"><component :is="providerTypeIcons[step.type]" :size="15" aria-hidden="true" /></span>
      <span class="pipeline-label">{{ step.label }}</span>
      <strong :title="step.name || t('templateForm.noProvider')">{{ step.name || '—' }}</strong>
    </li>
  </ol>
</template>

<style scoped>
/* Four stages joined by a rail: a filled node means a provider is bound to that stage. */
.template-mini-pipeline { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); }
.template-mini-pipeline > li { position: relative; display: grid; justify-items: center; gap: 3px; min-width: 0; padding: 0 4px; text-align: center; }
.template-mini-pipeline > li + li::before { content: ''; position: absolute; top: 16px; right: calc(50% + 20px); width: calc(100% - 40px); height: 2px; border-radius: 2px; background: var(--border); }
.template-mini-pipeline > li[data-bound="true"] + li[data-bound="true"]::before { background: color-mix(in oklab, var(--ink, var(--studio-violet)) 60%, var(--border)); }
.pipeline-node { display: grid; place-items: center; width: 34px; height: 34px; margin-bottom: 3px; border: 1.5px dashed var(--border); border-radius: 50%; color: var(--muted-foreground); }
li[data-bound="true"] > .pipeline-node { border: 0; background: color-mix(in oklab, var(--ink, var(--studio-violet)) 16%, transparent); color: var(--ink, var(--studio-violet)); }
.pipeline-label { color: var(--muted-foreground); font-size: 10.5px; font-weight: 600; letter-spacing: 0.06em; text-transform: uppercase; }
.template-mini-pipeline strong { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12.5px; font-weight: 500; }
li[data-bound="false"] > strong { color: var(--muted-foreground); }
</style>

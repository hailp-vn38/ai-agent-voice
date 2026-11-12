<script setup lang="ts">
import { ArrowUpRight, Globe2, Layers3, MessageSquareText, Users } from '@lucide/vue'
import { computed } from 'vue'

import TemplateActionsMenu from '@/components/templates/TemplateActionsMenu.vue'
import TemplatePipelineSummary from '@/components/templates/TemplatePipelineSummary.vue'
import { useI18n } from '@/composables/useI18n'
import type { AgentTemplate } from '@/domain/admin'
import { identityHue, monogram } from '@/lib/visualIdentity'

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
  <article class="dir-card template-card" :style="{ '--hue': identityHue(template.id) }">
    <div class="dir-card-head">
      <span class="template-icon" aria-hidden="true"><Layers3 :size="22" /></span>
      <div class="dir-card-title">
        <h3 class="template-heading"><button type="button" :aria-label="t('templatesBoard.open', { name: template.name })" @click="emit('open')">{{ template.name }}</button></h3>
        <p><Globe2 :size="12" aria-hidden="true" />{{ template.language || t('templateCard.noLanguage') }}</p>
      </div>
      <div class="dir-card-raise" @click.stop>
        <TemplateActionsMenu :template="template" show-edit @edit="emit('edit')" @view-details="emit('open')" @link-to-agent="emit('linkToAgent')" @copy="emit('copy')" @delete="emit('delete')" />
      </div>
    </div>
    <p class="dir-card-desc">{{ template.description || t('common.noDescription') }}</p>
    <blockquote class="template-prompt" :data-empty="!template.prompt">
      <MessageSquareText :size="14" aria-hidden="true" />
      <p>{{ template.prompt || t('templateForm.reviewNoPrompt') }}</p>
    </blockquote>
    <TemplatePipelineSummary :template="template" :provider-name-by-id="providerNameById" />
    <div class="dir-card-foot">
      <span v-if="used" class="template-agents" aria-hidden="true">
        <i v-for="name in previewedAgentNames" :key="name" :style="{ '--hue': identityHue(name) }" :title="name">{{ monogram(name, 'A') }}</i>
        <i v-if="overflowCount > 0" class="template-agents-more">+{{ overflowCount }}</i>
      </span>
      <span v-else><Users :size="14" aria-hidden="true" /></span>
      <span class="template-usage">
        <strong>{{ t('count.agents', { count: agentCount }) }}</strong>
        <small v-if="used && previewedAgentNames.length">{{ previewedAgentNames.join(', ') }}<template v-if="overflowCount > 0">, {{ t('count.more', { count: overflowCount }) }}</template></small>
        <small v-else>{{ used ? t('templateCard.linked') : t('templateCard.unused') }}</small>
      </span>
      <ArrowUpRight class="dir-card-arrow dir-card-push" :size="18" aria-hidden="true" />
    </div>
  </article>
</template>

<style scoped src="../admin/studio-directory-cards.css"></style>
<style scoped src="./template-card.css"></style>

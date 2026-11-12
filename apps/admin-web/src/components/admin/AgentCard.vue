<script setup lang="ts">
import { Layers, MonitorSmartphone, Plus, ArrowUpRight } from '@lucide/vue'

import { providerTypeIcons } from '@/lib/providerTypeIcons'
import { identityHue, monogram } from '@/lib/visualIdentity'
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
  <article class="dir-card agent-card group cursor-pointer" :style="{ '--hue': identityHue(agent.id) }">
    <button data-agent-card-open type="button" class="dir-card-hit absolute inset-0" :aria-label="agent.name" @click="emit('open')" />
    <div class="dir-card-head">
      <span class="dir-avatar" aria-hidden="true">{{ monogram(agent.name, 'A') }}</span>
      <div class="dir-card-title">
        <h3>{{ agent.name }}</h3>
        <code>@{{ agent.id }}</code>
      </div>
      <ArrowUpRight :size="18" class="dir-card-arrow" aria-hidden="true" />
    </div>
    <p class="dir-card-desc">{{ agent.description || t('common.noDescription') }}</p>

    <div class="agent-template">
      <div class="agent-template-head">
        <span><Layers :size="12" aria-hidden="true" />{{ t('templateUsage.defaultTemplate') }}</span>
        <em v-if="defaultTemplate?.language" class="dir-tag">{{ defaultTemplate.language }}</em>
      </div>
      <strong :data-empty="!defaultTemplate">{{ defaultTemplate?.name || t('agents.noTemplate') }}</strong>
      <ul v-if="defaultTemplate" class="agent-providers">
        <li v-for="provider in defaultTemplateProviders" :key="provider.id">
          <component :is="providerTypeIcons[provider.type]" :size="12" aria-hidden="true" /><span>{{ provider.name }}</span>
        </li>
        <li v-if="!defaultTemplateProviders.length">{{ t('studio.pipeline.default') }}</li>
      </ul>
    </div>

    <div class="dir-card-foot">
      <span><Layers :size="14" aria-hidden="true" />{{ t('count.templates', { count: templateCount }) }}</span>
      <span><MonitorSmartphone :size="14" aria-hidden="true" />{{ t('count.devices', { count: deviceCount }) }}</span>
      <Button data-agent-card-add-device class="dir-card-raise dir-card-push" size="sm" variant="outline" @click.stop="emit('addDevice')">
        <Plus class="size-3.5" aria-hidden="true" />{{ t('agentDevices.add') }}
      </Button>
    </div>
  </article>
</template>

<style scoped src="./studio-directory-cards.css"></style>

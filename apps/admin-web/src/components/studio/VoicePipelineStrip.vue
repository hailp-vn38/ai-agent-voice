<script setup lang="ts">
import { AudioLines, BrainCircuit, Mic, Volume2 } from '@lucide/vue'
import { computed } from 'vue'

import type { AgentTemplate, ProviderInstance } from '@/domain/admin'
import { useI18n } from '@/composables/useI18n'

type PipelineType = 'vad' | 'asr' | 'llm' | 'tts'
const props = defineProps<{ template: AgentTemplate; providers: ProviderInstance[] }>()
const { t } = useI18n()

const nodes = computed(() => {
  const specs = [
    { type: 'vad', label: 'VAD', icon: AudioLines },
    { type: 'asr', label: 'ASR', icon: Mic },
    { type: 'llm', label: 'LLM', icon: BrainCircuit },
    { type: 'tts', label: 'TTS', icon: Volume2 },
  ] as const
  return specs.map(({ type, label, icon }) => {
    const boundId = props.template.providerBindings[type as PipelineType]
    const provider = boundId ? props.providers.find((item) => item.id === boundId) : undefined
    return {
      type,
      label,
      icon,
      name: provider?.name ?? (boundId ? boundId : t('studio.pipeline.default')),
      bound: Boolean(boundId),
    }
  })
})
</script>

<template>
  <div class="grid grid-cols-2 gap-2 lg:grid-cols-4" :aria-label="t('studio.pipeline.title')">
    <div
      v-for="node in nodes"
      :key="node.type"
      class="rounded-xl border border-border/70 bg-surface p-3"
    >
      <div class="mb-3 flex items-center gap-2">
        <component :is="node.icon" class="size-4 text-studio-cyan" aria-hidden="true" />
        <span class="text-xs font-semibold tracking-wide">{{ node.label }}</span>
      </div>
      <p class="truncate text-xs" :class="node.bound ? 'text-foreground' : 'text-muted-foreground'" :title="node.name">
        {{ node.name }}
      </p>
    </div>
  </div>
</template>

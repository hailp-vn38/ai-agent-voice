<script setup lang="ts">
import { ArrowRight, AudioLines, Mic, Pause, Play, Sparkles, Volume2 } from '@lucide/vue'
import { computed, ref, useId } from 'vue'
import EmptyProviderNode from '@/components/pipeline/EmptyProviderNode.vue'
import ProviderNode from '@/components/pipeline/ProviderNode.vue'
import { useI18n } from '@/composables/useI18n'
import { templateCandidateProviders, type AgentTemplate, type ProviderInstance, type ProviderType } from '@/domain/admin'

const props = defineProps<{
  template: AgentTemplate
  providers: ProviderInstance[]
  sharedTemplates: (type: ProviderType, providerId: string) => string[]
}>()
const emit = defineEmits<{
  openProvider: [provider: ProviderInstance]
  editProvider: [provider: ProviderInstance]
  unlink: [type: ProviderType]
  select: [type: ProviderType, providerId: string]
}>()
const { t } = useI18n()
const headingId = useId()
const paused = ref(false)
const types = ['vad', 'asr', 'llm', 'tts'] as const
const stages = computed(() => types.map(type => {
  const provider = props.providers.find(item => item.id === props.template.providerBindings[type] && item.type === type)
  return { type, provider, candidates: templateCandidateProviders(props.providers, type), shared: provider ? props.sharedTemplates(type, provider.id) : [] }
}))
const boundCount = computed(() => stages.value.filter(stage => stage.provider).length)
const bars = Array.from({ length: 25 }, (_, i) => 7 + (Math.sin(i * 0.87) * 0.5 + 0.5) * 36)
</script>

<template>
  <section class="voice-pipeline" :class="{ 'is-paused': paused }" :aria-labelledby="headingId">
    <header class="pipeline-heading">
      <div>
        <p class="pipeline-kicker"><Sparkles :size="12" aria-hidden="true" />{{ t('agentStudio.eyebrow') }}</p>
        <h2 :id="headingId">{{ t('pipeline.heading') }}<span class="pipeline-heading-dot" aria-hidden="true" /></h2>
        <p class="pipeline-caption">{{ t('agentStudio.caption') }}</p>
      </div>
      <div class="pipeline-controls">
        <span class="pipeline-coverage"><i aria-hidden="true" />{{ t('count.slots', { bound: boundCount, total: 4 }) }}</span>
        <button type="button" class="pipeline-motion" :aria-label="t(paused ? 'agentStudio.play' : 'agentStudio.pause')" :title="t(paused ? 'agentStudio.play' : 'agentStudio.pause')" :aria-pressed="paused" @click="paused = !paused">
          <Play v-if="paused" :size="14" aria-hidden="true" /><Pause v-else :size="14" aria-hidden="true" />
        </button>
      </div>
    </header>

    <div class="pipeline-endpoints">
      <span><Mic :size="13" aria-hidden="true" />{{ t('agentStudio.input') }}</span>
      <div class="pipeline-endpoint-rule" aria-hidden="true"><i /></div>
      <span>{{ t('agentStudio.output') }}<Volume2 :size="13" aria-hidden="true" /></span>
    </div>

    <div class="pipeline-stages">
      <div v-for="(stage, index) in stages" :key="stage.type" class="pipeline-stage" :data-stage="stage.type" :data-configured="Boolean(stage.provider)" :data-ready="stage.provider?.status === 'ready'" :style="{ '--stage-index': index }">
        <div class="stage-topline"><span class="stage-number">0{{ index + 1 }}</span><span>{{ stage.type.toUpperCase() }}</span><i aria-hidden="true" /></div>
        <div class="stage-art" aria-hidden="true">
          <svg viewBox="0 0 240 96" fill="none" focusable="false">
            <path d="M12 48 H228" class="art-baseline" />
            <template v-if="stage.type === 'vad'">
              <rect x="55" y="12" width="130" height="72" rx="14" class="art-window" />
              <rect v-for="(bar, i) in bars" :key="i" :x="30 + i * 7.5" :y="48 - bar / 2" width="3" :height="bar" rx="1.5" class="art-wave" :style="{ '--phase': `${i * -0.11}s` }" />
              <path d="M55 24 V72" class="art-scan" />
            </template>
            <template v-else-if="stage.type === 'asr'">
              <path d="M24 48 h8 l5-12 6 24 7-31 6 35 5-16 h18" class="art-stroke art-transcription-wave" />
              <path d="M91 48 H107" class="art-stroke" />
              <rect x="117" y="15" width="98" height="66" rx="12" class="art-window" />
              <path d="M133 33 h61 M133 48 h45 M133 63 h54" pathLength="100" class="art-text" />
              <path d="M196 42 V55" class="art-cursor" />
            </template>
            <template v-else-if="stage.type === 'llm'">
              <ellipse cx="120" cy="48" rx="73" ry="30" class="art-orbit" />
              <ellipse cx="120" cy="48" rx="46" ry="22" transform="rotate(-27 120 48)" class="art-orbit" />
              <path d="M46 48 92 29 120 48 148 25 194 48 147 70 120 48 89 72Z" class="art-neural-links" />
              <circle v-for="(p, i) in [[46,48],[92,29],[148,25],[194,48],[147,70],[89,72]]" :key="i" :cx="p[0]" :cy="p[1]" r="3" class="art-neuron" :style="{ '--phase': `${i * -0.55}s` }" />
              <circle cx="120" cy="48" r="23" class="art-core-halo" />
              <path d="M120 29 Q123 44 139 48 Q123 52 120 67 Q117 52 101 48 Q117 44 120 29Z" class="art-core" />
              <circle cx="120" cy="48" r="3" class="art-neuron" />
            </template>
            <template v-else>
              <path d="m46 40 12-10v36L46 56H35V40Z" class="art-speaker" />
              <path d="M68 35 Q85 48 68 61 M77 25 Q105 48 77 71" class="art-sound-rings" />
              <rect v-for="(bar, i) in bars.slice(0, 18)" :key="i" :x="108 + i * 6" :y="48 - bar / 2" width="2.5" :height="bar" rx="1.2" class="art-wave" :style="{ '--phase': `${i * -0.14}s` }" />
            </template>
          </svg>
        </div>
        <div class="stage-copy"><h3>{{ t(`agentStudio.${stage.type}.title`) }}</h3><p>{{ t(`agentStudio.${stage.type}.description`) }}</p></div>
        <div class="stage-provider">
          <ProviderNode v-if="stage.provider" :type="stage.type" :provider="stage.provider" :shared-with="stage.shared" @open="emit('openProvider', $event)" @edit="emit('editProvider', $event)" @unlink="emit('unlink', $event)" />
          <EmptyProviderNode v-else :type="stage.type" :candidates="stage.candidates" @select="(type, id) => emit('select', type, id)" />
        </div>
        <div v-if="index < 3" class="stage-connector" :data-connected="stage.provider?.status === 'ready' && stages[index + 1]?.provider?.status === 'ready'" aria-hidden="true"><i /><ArrowRight :size="13" /></div>
      </div>
    </div>
    <footer class="pipeline-footnote"><AudioLines :size="13" aria-hidden="true" /><span>{{ t('agentStudio.illustration') }}</span><span class="pipeline-topology">VAD <ArrowRight :size="10" /> ASR <ArrowRight :size="10" /> LLM <ArrowRight :size="10" /> TTS</span></footer>
  </section>
</template>

<style scoped src="./voice-pipeline-studio.css"></style>

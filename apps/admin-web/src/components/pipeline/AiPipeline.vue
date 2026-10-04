<script setup lang="ts">
import { computed } from 'vue'

import EmptyProviderNode from '@/components/pipeline/EmptyProviderNode.vue'
import ProviderNode from '@/components/pipeline/ProviderNode.vue'
import { useI18n } from '@/composables/useI18n'
import {
  providerTypes,
  type AgentTemplate,
  type ProviderInstance,
  type ProviderType,
} from '@/domain/admin'

const props = defineProps<{
  template: AgentTemplate
  providers: ProviderInstance[]
  /** Names of the other templates binding the same provider instance. */
  sharedTemplates: (type: ProviderType, providerId: string) => string[]
}>()

const emit = defineEmits<{
  openProvider: [provider: ProviderInstance]
  editProvider: [provider: ProviderInstance]
  unlink: [type: ProviderType]
  select: [type: ProviderType, providerId: string]
}>()

const { t } = useI18n()

/** Provider currently bound to a type, resolved from the template's binding map. */
function providerFor(type: ProviderType) {
  const providerId = props.template.providerBindings[type]
  if (!providerId) return undefined
  return props.providers.find((provider) => provider.id === providerId)
}

function candidatesFor(type: ProviderType) {
  return props.providers.filter((provider) => provider.type === type)
}

function sharedFor(type: ProviderType) {
  const provider = providerFor(type)
  return provider ? props.sharedTemplates(type, provider.id) : []
}

// Explicit topology: VAD → ASR → LLM, with Vision branching off the LLM and TTS continuing the audio path.
const vad = computed(() => providerFor('vad'))
const asr = computed(() => providerFor('asr'))
const llm = computed(() => providerFor('llm'))
const tts = computed(() => providerFor('tts'))
const vision = computed(() => providerFor('vision'))

const boundCount = computed(() => providerTypes.filter((type) => providerFor(type)).length)
</script>

<template>
  <section
    class="rounded-xl border border-border/70 bg-card p-4 sm:p-5"
    aria-labelledby="ai-pipeline-heading"
  >
    <header class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
      <h2 id="ai-pipeline-heading" class="text-base font-semibold tracking-tight">
          {{ t('pipeline.heading') }}
        </h2>
      <p class="text-xs text-muted-foreground">
        {{ t('count.slots', { bound: boundCount, total: providerTypes.length }) }}
      </p>
    </header>
    <p class="mt-1 text-sm text-muted-foreground">
      {{ t('pipeline.description') }}
    </p>

    <p class="mt-5 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">{{ t('pipeline.input') }}</p>

    <div class="mt-2 flex flex-col">
      <ProviderNode
        v-if="vad"
        type="vad"
        :provider="vad"
        :shared-with="sharedFor('vad')"
        @open="emit('openProvider', $event)"
        @edit="emit('editProvider', $event)"
        @unlink="emit('unlink', $event)"
      />
      <EmptyProviderNode
        v-else
        type="vad"
        :candidates="candidatesFor('vad')"
        @select="(type, providerId) => emit('select', type, providerId)"
      />

      <div class="flex h-7 justify-center" aria-hidden="true">
        <span class="flex w-4 flex-col items-center">
          <span class="w-px flex-1 bg-foreground/25" />
          <svg viewBox="0 0 16 8" class="size-4 shrink-0 text-foreground/25" fill="none" aria-hidden="true">
            <path d="M3 1.5 8 6.5l5-5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </span>
      </div>

      <ProviderNode
        v-if="asr"
        type="asr"
        :provider="asr"
        :shared-with="sharedFor('asr')"
        @open="emit('openProvider', $event)"
        @edit="emit('editProvider', $event)"
        @unlink="emit('unlink', $event)"
      />
      <EmptyProviderNode
        v-else
        type="asr"
        :candidates="candidatesFor('asr')"
        @select="(type, providerId) => emit('select', type, providerId)"
      />

      <div class="flex h-7 justify-center" aria-hidden="true">
        <span class="flex w-4 flex-col items-center">
          <span class="w-px flex-1 bg-foreground/25" />
          <svg viewBox="0 0 16 8" class="size-4 shrink-0 text-foreground/25" fill="none" aria-hidden="true">
            <path d="M3 1.5 8 6.5l5-5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </span>
      </div>

      <!-- LLM, with Vision as a side capability branch, both cells stretch so the horizontal link lines up. -->
      <div class="grid grid-cols-1 items-stretch gap-3 sm:grid-cols-[minmax(0,1fr)_2.5rem_minmax(0,1fr)] sm:gap-0">
        <div class="flex flex-col items-center">
          <span class="h-4 w-px bg-foreground/25" aria-hidden="true" />
          <ProviderNode
            v-if="llm"
            class="h-full w-full"
            type="llm"
            :provider="llm"
            :shared-with="sharedFor('llm')"
            @open="emit('openProvider', $event)"
            @edit="emit('editProvider', $event)"
            @unlink="emit('unlink', $event)"
          />
          <EmptyProviderNode
            v-else
            class="h-full w-full"
            type="llm"
            :candidates="candidatesFor('llm')"
            @select="(type, providerId) => emit('select', type, providerId)"
          />
        </div>

        <div class="hidden items-center sm:flex" aria-hidden="true">
          <span class="h-px w-full bg-foreground/25" />
          <svg viewBox="0 0 16 8" class="-ml-px size-4 shrink-0 text-foreground/25" fill="none" aria-hidden="true">
            <path d="M3 1.5 8 6.5l5-5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </div>

        <div class="flex flex-col items-center">
          <span class="hidden h-4 w-px bg-foreground/25 sm:block" aria-hidden="true" />
          <ProviderNode
            v-if="vision"
            class="h-full w-full"
            type="vision"
            :provider="vision"
            :shared-with="sharedFor('vision')"
            @open="emit('openProvider', $event)"
            @edit="emit('editProvider', $event)"
            @unlink="emit('unlink', $event)"
          />
          <EmptyProviderNode
            v-else
            class="h-full w-full"
            type="vision"
            :candidates="candidatesFor('vision')"
            @select="(type, providerId) => emit('select', type, providerId)"
          />
        </div>
      </div>

      <div class="flex h-7 justify-center" aria-hidden="true">
        <span class="flex w-4 flex-col items-center">
          <span class="w-px flex-1 bg-foreground/25" />
          <svg viewBox="0 0 16 8" class="size-4 shrink-0 text-foreground/25" fill="none" aria-hidden="true">
            <path d="M3 1.5 8 6.5l5-5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </span>
      </div>

      <ProviderNode
        v-if="tts"
        type="tts"
        :provider="tts"
        :shared-with="sharedFor('tts')"
        @open="emit('openProvider', $event)"
        @edit="emit('editProvider', $event)"
        @unlink="emit('unlink', $event)"
      />
      <EmptyProviderNode
        v-else
        type="tts"
        :candidates="candidatesFor('tts')"
        @select="(type, providerId) => emit('select', type, providerId)"
      />
    </div>

    <p class="mt-5 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">{{ t('pipeline.output') }}</p>
  </section>
</template>
<script setup lang="ts">
import { ChevronRight } from '@lucide/vue'
import { computed } from 'vue'

import { useI18n } from '@/composables/useI18n'
import { providerTypes, type AgentTemplate } from '@/domain/admin'

const props = defineProps<{
  template: AgentTemplate
  providerNameById: (providerId?: string) => string
}>()

/** Voice path in pipeline order. Vision is an LLM capability, so it branches below. */
const { t, providerTypeLabel } = useI18n()

const chain = computed(() =>
  providerTypes
    .filter((type) => type !== 'vision' && props.template.providerBindings[type])
    .map((type) => ({
      type,
      label: providerTypeLabel(type),
      name: props.providerNameById(props.template.providerBindings[type]),
    })),
)

const vision = computed(() => {
  const providerId = props.template.providerBindings.vision
  return providerId ? props.providerNameById(providerId) : undefined
})
</script>

<template>
  <p v-if="chain.length === 0 && !vision" class="text-xs text-muted-foreground">
    {{ t('templateSummary.noProviders') }}
  </p>

  <div v-else class="space-y-1.5">
    <p v-if="chain.length" class="flex flex-wrap items-center gap-x-1.5 gap-y-1 text-xs">
      <template v-for="(step, index) in chain" :key="step.type">
        <ChevronRight
          v-if="index > 0"
          class="size-3 shrink-0 text-muted-foreground/60"
          aria-hidden="true"
        />
        <span class="inline-flex min-w-0 items-baseline gap-1">
          <span class="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
            {{ step.label }}
          </span>
          <span class="max-w-32 truncate font-medium text-foreground" :title="step.name">
            {{ step.name }}
          </span>
        </span>
      </template>
    </p>

    <p v-if="vision" class="flex items-center gap-1.5 pl-3 text-xs">
      <span class="font-mono text-muted-foreground/70" aria-hidden="true">└─</span>
      <span class="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">Vision</span>
      <span class="max-w-32 truncate font-medium text-foreground" :title="vision">{{ vision }}</span>
    </p>
  </div>
</template>

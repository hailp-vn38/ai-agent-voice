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
    .filter((type) => props.template.providerBindings[type])
    .map((type) => ({
      type,
      label: providerTypeLabel(type),
      name: props.providerNameById(props.template.providerBindings[type]),
    })),
)
</script>

<template>
  <p v-if="chain.length === 0" class="text-xs text-muted-foreground">
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
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'

import ProviderSelector from '@/components/pipeline/ProviderSelector.vue'
import { useI18n } from '@/composables/useI18n'
import type { ProviderInstance, ProviderType } from '@/domain/admin'
import { providerTypeIcons } from '@/lib/providerTypeIcons'

const props = defineProps<{
  type: ProviderType
  candidates: ProviderInstance[]
}>()

const emit = defineEmits<{ select: [type: ProviderType, providerId: string] }>()

const { t, providerTypeLabel } = useI18n()

const icon = computed(() => providerTypeIcons[props.type])
</script>

<template>
  <article class="rounded-lg border border-dashed border-border/80 bg-surface/50 px-3.5 py-3">
    <p class="flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
      <component :is="icon" class="size-3.5" aria-hidden="true" />
      {{ providerTypeLabel(type) }}
    </p>

    <p class="mt-2 text-sm font-medium text-muted-foreground">{{ t('pipeline.emptyNode') }}</p>

    <ProviderSelector
      class="mt-3"
      :type="type"
      :candidates="candidates"
      @select="emit('select', type, $event)"
    />
  </article>
</template>

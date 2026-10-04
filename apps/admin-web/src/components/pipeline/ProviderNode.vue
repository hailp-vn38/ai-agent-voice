<script setup lang="ts">
import { computed } from 'vue'

import ProviderActionsMenu from '@/components/pipeline/ProviderActionsMenu.vue'
import { useI18n } from '@/composables/useI18n'
import type { ProviderInstance, ProviderType } from '@/domain/admin'
import { providerTypeIcons } from '@/lib/providerTypeIcons'

const props = defineProps<{
  type: ProviderType
  provider: ProviderInstance
  /** Names of the other templates that also bind this provider instance. */
  sharedWith?: string[]
}>()

const emit = defineEmits<{
  open: [provider: ProviderInstance]
  edit: [provider: ProviderInstance]
  unlink: [type: ProviderType]
}>()

const { t, providerTypeLabel, providerStatusLabel } = useI18n()

const icon = computed(() => providerTypeIcons[props.type])
const statusLabel = computed(() => providerStatusLabel(props.provider.status))
const statusTone = computed(() => {
  if (props.provider.status === 'ready') return 'bg-success'
  if (props.provider.status === 'error') return 'bg-danger'
  return 'bg-muted-foreground/50'
})
</script>

<template>
  <article class="group relative rounded-lg border border-border/70 bg-surface transition-colors hover:border-ring/50">
    <button
      type="button"
      class="flex w-full flex-col gap-2 rounded-lg px-3.5 py-3 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
      :aria-label="t('pipeline.openProvider', { type: providerTypeLabel(type), name: provider.name })"
      @click="emit('open', provider)"
    >
      <span class="flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        <component :is="icon" class="size-3.5" aria-hidden="true" />
        {{ providerTypeLabel(type) }}
      </span>

      <span class="truncate text-sm font-semibold leading-tight text-foreground">{{ provider.name }}</span>

      <span class="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
        <span class="truncate font-mono">{{ provider.adapter }}</span>
        <span class="inline-flex shrink-0 items-center gap-1.5">
          <span class="size-1.5 rounded-full" :class="statusTone" aria-hidden="true" />
          {{ statusLabel }}
        </span>
      </span>
    </button>

    <ProviderActionsMenu
      class="absolute right-2 top-2"
      :type="type"
      :provider="provider"
      :shared-with="sharedWith"
      @view="emit('open', provider)"
      @edit="emit('edit', provider)"
      @unlink="emit('unlink', type)"
    />
  </article>
</template>

<script setup lang="ts">
import { LayoutTemplate } from '@lucide/vue'

import { useI18n } from '@/composables/useI18n'
import type { ProviderUsageEntry } from '@/domain/admin'

defineProps<{
  providerName: string
  usage: ProviderUsageEntry[]
}>()

const emit = defineEmits<{ navigate: [] }>()

const { t } = useI18n()
</script>

<template>
  <div class="rounded-md border border-border/70 bg-surface/60 p-2">
    <p class="px-1 pb-1.5 text-[11px] text-muted-foreground">
      {{ t('providers.usageTitle', { name: providerName }) }}
    </p>

    <p v-if="!usage.length" class="px-1 text-xs text-muted-foreground">
      {{ t('providers.usageEmpty') }}
    </p>

    <ul v-else class="space-y-0.5">
      <li v-for="entry in usage" :key="entry.templateId">
        <button
          type="button"
          class="flex w-full items-start gap-2 rounded px-1 py-1 text-left transition-colors hover:bg-accent focus-visible:ring-ring focus-visible:outline-none focus-visible:ring-2"
          @click.stop="emit('navigate')"
        >
          <LayoutTemplate class="mt-0.5 size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
          <span class="min-w-0">
            <span class="block truncate text-xs font-medium">{{ entry.templateName }}</span>
            <span class="block truncate text-[11px] text-muted-foreground">
              {{ [entry.language, ...entry.agentNames].filter(Boolean).join(' · ') || '—' }}
            </span>
          </span>
        </button>
      </li>
    </ul>
  </div>
</template>
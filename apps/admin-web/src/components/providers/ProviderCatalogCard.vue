<script setup lang="ts">
import { computed, ref } from 'vue'

import ProviderActionsMenu from '@/components/providers/ProviderActionsMenu.vue'
import ProviderUsageList from '@/components/providers/ProviderUsageList.vue'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { useI18n } from '@/composables/useI18n'
import type { ProviderInstance, ProviderUsageEntry } from '@/domain/admin'
import { providerTypeIcons } from '@/lib/providerTypeIcons'

const props = defineProps<{
  provider: ProviderInstance
  /** Derived from the template bindings on every render, never persisted. */
  usage: ProviderUsageEntry[]
}>()

const emit = defineEmits<{
  open: []
  edit: []
  link: []
  duplicate: []
  delete: []
}>()

const { t, providerTypeLabel, providerStatusLabel } = useI18n()

const usageOpen = ref(false)

const typeIcon = computed(() => providerTypeIcons[props.provider.type])

const usageCount = computed(() => props.usage.length)

const PREVIEW_LIMIT = 2

const usagePreview = computed(() => {
  const names = props.usage.slice(0, PREVIEW_LIMIT).map((entry) => entry.templateName)
  const overflow = usageCount.value - names.length
  return overflow > 0 ? `${names.join(' · ')} · ${t('count.more', { count: overflow })}` : names.join(' · ')
})

const statusTone = computed(() => {
  if (props.provider.status === 'error') return 'text-danger'
  if (props.provider.status === 'disabled') return 'text-muted-foreground'
  return 'text-success'
})
</script>

<template>
  <Card
    class="group flex h-full cursor-pointer flex-col transition-colors hover:border-foreground/20 hover:bg-accent/40"
    @click="emit('open')"
  >
    <CardHeader class="gap-2.5 pb-3">
      <div class="flex items-start justify-between gap-3">
        <div class="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
          <component :is="typeIcon" class="size-3.5 shrink-0" aria-hidden="true" />
          <span class="font-medium tracking-wide uppercase">{{ providerTypeLabel(provider.type) }}</span>
        </div>
        <div class="flex shrink-0 items-center gap-1">
          <span
            class="inline-flex items-center gap-1.5 text-xs"
            :class="statusTone"
          >
            <span class="size-1.5 rounded-full bg-current" aria-hidden="true" />
            {{ providerStatusLabel(provider.status) }}
          </span>
          <ProviderActionsMenu
            :provider="provider"
            @view="emit('open')"
            @edit="emit('edit')"
            @link="emit('link')"
            @duplicate="emit('duplicate')"
            @delete="emit('delete')"
          />
        </div>
      </div>

      <div class="min-w-0">
        <CardTitle class="truncate text-base font-semibold">{{ provider.name }}</CardTitle>
        <p class="mt-1 truncate font-mono text-xs text-muted-foreground">{{ provider.adapter }}</p>
      </div>
    </CardHeader>

    <CardContent class="flex flex-1 flex-col gap-3">
      <div class="min-w-0">
        <p class="text-[11px] tracking-wide text-muted-foreground uppercase">
          {{ t('providers.model') }}
        </p>
        <p class="mt-0.5 truncate font-mono text-xs">
          {{ provider.model || t('providers.modelMissing') }}
        </p>
      </div>

      <p
        v-if="provider.description"
        class="line-clamp-2 text-xs leading-relaxed text-muted-foreground"
      >
        {{ provider.description }}
      </p>

      <div class="mt-auto space-y-2 border-t pt-3">
        <button
          type="button"
          class="block w-full rounded-md text-left text-xs transition-colors hover:text-foreground focus-visible:ring-ring focus-visible:outline-none focus-visible:ring-2"
          :class="usageCount ? 'text-muted-foreground' : 'text-muted-foreground/70'"
          :aria-expanded="usageOpen"
          :aria-label="t('providers.usageToggleLabel', { name: provider.name })"
          :disabled="!usageCount"
          @click.stop="usageOpen = !usageOpen"
        >
          {{ usageCount ? t('providers.usedBy', { count: usageCount }) : t('providers.unused') }}
        </button>
        <p v-if="usageCount && !usageOpen" class="truncate text-xs" :title="usagePreview">
          {{ usagePreview }}
        </p>

        <ProviderUsageList
          v-if="usageOpen"
          :provider-name="provider.name"
          :usage="usage"
          @navigate="emit('open')"
        />
      </div>
    </CardContent>
  </Card>
</template>

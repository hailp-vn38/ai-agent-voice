<script setup lang="ts">
import { computed } from 'vue'

import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { useI18n } from '@/composables/useI18n'
import { providerTypes } from '@/domain/admin'
import { providerTypeIcons } from '@/lib/providerTypeIcons'

const props = defineProps<{
  /** Providers already narrowed by search and status, so the counts stay honest. */
  counts: Record<string, number>
}>()

const activeType = defineModel<string>({ required: true })

const { t, providerTypeLabel } = useI18n()

const ALL = 'all'

/** Pipeline order, so the tabs read left to right like the AI pipeline does. */
const tabs = computed(() => [
  { value: ALL, label: t('common.all'), count: props.counts[ALL] ?? 0, icon: undefined },
  ...providerTypes.map((type) => ({
    value: type,
    label: providerTypeLabel(type),
    count: props.counts[type] ?? 0,
    icon: providerTypeIcons[type],
  })),
])
</script>

<template>
  <Tabs v-model="activeType">
    <TabsList :aria-label="t('providers.typeFilter')" class="max-w-full">
      <TabsTrigger v-for="tab in tabs" :key="tab.value" :value="tab.value" class="gap-1.5">
        <component :is="tab.icon" v-if="tab.icon" class="size-4 shrink-0" aria-hidden="true" />
        <span class="shrink-0">{{ tab.label }}</span>
        <span
          class="rounded-full bg-muted px-1.5 text-xs tabular-nums text-muted-foreground"
          :class="activeType === tab.value ? 'bg-background/70' : undefined"
        >
          {{ tab.count }}
        </span>
      </TabsTrigger>
    </TabsList>
  </Tabs>
</template>
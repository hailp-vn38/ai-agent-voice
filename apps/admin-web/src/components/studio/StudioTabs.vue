<script setup lang="ts">
import { useI18n } from '@/composables/useI18n'

type AgentStudioTab = 'studio' | 'tools' | 'speakers' | 'devices'

defineProps<{ modelValue: AgentStudioTab }>()
const emit = defineEmits<{ 'update:modelValue': [tab: AgentStudioTab] }>()
const { t } = useI18n()
const items = [
  { key: 'studio', label: 'studio.tab.studio' },
  { key: 'tools', label: 'studio.tab.tools' },
  { key: 'speakers', label: 'studio.tab.speakers' },
  { key: 'devices', label: 'studio.tab.devices' },
] as const
</script>

<template>
  <div class="flex gap-1 overflow-x-auto border-b border-border/70" role="tablist" :aria-label="t('studio.tab.label')">
    <button
      v-for="item in items"
      :key="item.key"
      type="button"
      role="tab"
      :id="`studio-tab-${item.key}`"
      :aria-controls="`studio-panel-${item.key}`"
      :aria-selected="modelValue === item.key"
      :tabindex="modelValue === item.key ? 0 : -1"
      class="min-w-max border-b-2 px-4 py-3 text-sm font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-ring"
      :class="modelValue === item.key ? 'border-studio-violet text-foreground' : 'border-transparent text-muted-foreground hover:text-foreground'"
      @click="emit('update:modelValue', item.key)"
      @keydown.left.prevent="emit('update:modelValue', items[(items.findIndex((entry) => entry.key === modelValue) + items.length - 1) % items.length].key)"
      @keydown.right.prevent="emit('update:modelValue', items[(items.findIndex((entry) => entry.key === modelValue) + 1) % items.length].key)"
    >{{ t(item.label) }}</button>
  </div>
</template>

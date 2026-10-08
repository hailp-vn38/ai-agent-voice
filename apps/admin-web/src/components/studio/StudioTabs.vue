<script setup lang="ts">
import { nextTick } from 'vue'
import { useI18n } from '@/composables/useI18n'

type AgentStudioTab = 'studio' | 'tools' | 'speakers' | 'devices'

const props = defineProps<{ modelValue: AgentStudioTab }>()
const emit = defineEmits<{ 'update:modelValue': [tab: AgentStudioTab] }>()
const { t } = useI18n()
const items = [
  { key: 'studio', label: 'studio.tab.studio' },
  { key: 'tools', label: 'studio.tab.tools' },
  { key: 'speakers', label: 'studio.tab.speakers' },
  { key: 'devices', label: 'studio.tab.devices' },
] as const

function activate(index: number) {
  const item = items[(index + items.length) % items.length]
  if (!item) return
  emit('update:modelValue', item.key)
  void nextTick(() => document.getElementById(`studio-tab-${item.key}`)?.focus())
}

function move(direction: number) {
  const current = items.findIndex((item) => item.key === props.modelValue)
  activate(current + direction)
}
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
      @keydown.left.prevent="move(-1)"
      @keydown.right.prevent="move(1)"
      @keydown.home.prevent="activate(0)"
      @keydown.end.prevent="activate(items.length - 1)"
    >{{ t(item.label) }}</button>
  </div>
</template>

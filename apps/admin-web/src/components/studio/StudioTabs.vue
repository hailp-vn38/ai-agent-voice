<script setup lang="ts">
import { nextTick } from 'vue'
import { Bell, Blocks, Cable, Mic, MonitorSmartphone } from '@lucide/vue'
import { useI18n } from '@/composables/useI18n'

type AgentStudioTab = 'studio' | 'tools' | 'speakers' | 'devices' | 'reminders'

const props = defineProps<{ modelValue: AgentStudioTab }>()
const emit = defineEmits<{ 'update:modelValue': [tab: AgentStudioTab] }>()
const { t } = useI18n()
const items = [
  { key: 'studio', label: 'studio.tab.studio', icon: Blocks },
  { key: 'tools', label: 'studio.tab.tools', icon: Cable },
  { key: 'speakers', label: 'studio.tab.speakers', icon: Mic },
  { key: 'devices', label: 'studio.tab.devices', icon: MonitorSmartphone },
  { key: 'reminders', label: 'reminders.title', icon: Bell },
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
  <div class="studio-tabs" role="tablist" :aria-label="t('studio.tab.label')">
    <button
      v-for="item in items"
      :key="item.key"
      type="button"
      role="tab"
      :id="`studio-tab-${item.key}`"
      :aria-controls="`studio-panel-${item.key}`"
      :aria-selected="modelValue === item.key"
      :tabindex="modelValue === item.key ? 0 : -1"
      @click="emit('update:modelValue', item.key)"
      @keydown.left.prevent="move(-1)"
      @keydown.right.prevent="move(1)"
      @keydown.home.prevent="activate(0)"
      @keydown.end.prevent="activate(items.length - 1)"
    ><component :is="item.icon" :size="14" aria-hidden="true" />{{ t(item.label) }}</button>
  </div>
</template>

<style scoped>
.studio-tabs { display: flex; gap: 5px; padding: 5px; overflow-x: auto; border: 1px solid var(--border); border-radius: 14px; background: var(--card); }
.studio-tabs button { display: flex; align-items: center; gap: 8px; flex-shrink: 0; padding: 9px 15px; border: 1px solid transparent; border-radius: 10px; color: var(--muted-foreground); font-size: 12px; font-weight: 550; cursor: pointer; transition: background .2s, color .2s; }
.studio-tabs button:hover { color: var(--foreground); background: var(--accent); }
.studio-tabs button[aria-selected="true"] { background: color-mix(in oklab, var(--studio-violet) 10%, var(--background)); border-color: color-mix(in oklab, var(--studio-violet) 17%, var(--border)); color: var(--studio-violet); }
.studio-tabs button:focus-visible { outline: 2px solid var(--ring); outline-offset: -2px; }
@media (prefers-reduced-motion: reduce) { .studio-tabs button { transition: none; } }
</style>

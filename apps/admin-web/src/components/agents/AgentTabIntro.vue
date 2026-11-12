<script setup lang="ts">
import { Bell, Cable, Mic, MonitorSmartphone } from '@lucide/vue'
import { useI18n } from '@/composables/useI18n'
defineProps<{ tone: 'tools' | 'speakers' | 'devices' | 'reminders' }>()
const { t } = useI18n()
const icons = { tools: Cable, speakers: Mic, devices: MonitorSmartphone, reminders: Bell }
</script>

<template>
  <header class="agent-tab-intro" :data-tone="tone">
    <span class="tab-intro-icon" aria-hidden="true"><component :is="icons[tone]" :size="22" /></span>
    <div class="tab-intro-copy"><p>AGENT / {{ tone.toUpperCase() }}</p><h2>{{ t(`agentBody.${tone}.title`) }}</h2><span>{{ t(`agentBody.${tone}.description`) }}</span></div>
    <div v-if="$slots.actions" class="tab-intro-actions"><slot name="actions" /></div>
    <svg class="tab-intro-art" viewBox="0 0 200 80" fill="none" aria-hidden="true" focusable="false">
      <path d="M10 40H190M60 12V68M140 12V68" stroke="currentColor" stroke-dasharray="2 7" />
      <circle cx="100" cy="40" r="27" stroke="currentColor" /><circle cx="100" cy="40" r="16" stroke="currentColor" />
      <path d="M10 40H190" pathLength="100" class="tab-intro-signal" stroke="currentColor" />
    </svg>
  </header>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { SyncStatus } from './sync'
import { syncStatusLabel } from './sync'

const props = defineProps<{ status: SyncStatus }>()
const emit = defineEmits<{ activate: [] }>()
const locale = computed(() => document.documentElement.lang === 'vi' ? 'vi' : 'en')
const label = computed(() => syncStatusLabel(props.status, locale.value))
</script>

<template>
  <button class="knowledge-sync-badge" type="button" aria-live="polite" @click="emit('activate')">
    <span class="knowledge-sync-badge__dot" :class="`is-${status.type}`" aria-hidden="true" />
    <span>{{ label }}</span>
  </button>
</template>

<style scoped>
.knowledge-sync-badge { display: inline-flex; align-items: center; gap: .45rem; max-width: 100%; padding: .35rem .55rem; border: 1px solid color-mix(in srgb, currentColor 20%, transparent); border-radius: 999px; color: inherit; background: transparent; cursor: pointer; }
.knowledge-sync-badge__dot { width: .5rem; height: .5rem; flex: none; border-radius: 50%; background: currentColor; opacity: .55; }
.knowledge-sync-badge__dot.is-syncing, .knowledge-sync-badge__dot.is-deferred { opacity: 1; animation: sync-pulse 1.2s ease-in-out infinite alternate; }
.knowledge-sync-badge__dot.is-idle { background: #1a9c5a; opacity: 1; }
.knowledge-sync-badge__dot.is-error, .knowledge-sync-badge__dot.is-conflicted { background: #d92d20; opacity: 1; }
@keyframes sync-pulse { to { opacity: .35; } }
</style>

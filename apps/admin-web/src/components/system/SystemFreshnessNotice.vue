<script setup lang="ts">
import { useI18n } from '@/composables/useI18n'
import type { Freshness } from '@/composables/useSystemLive'

const props = defineProps<{ freshness: Freshness }>()
const { t } = useI18n()
</script>

<template>
  <p
    v-if="props.freshness !== 'fresh'"
    role="status"
    class="text-xs text-muted-foreground"
    :data-freshness="props.freshness"
  >
    {{ t(props.freshness === 'stale' ? 'system.stale' : 'system.unavailable') }}
  </p>
</template>

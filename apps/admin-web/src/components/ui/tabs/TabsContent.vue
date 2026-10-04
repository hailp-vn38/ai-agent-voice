<script setup lang="ts">
import { inject } from 'vue'

import { TABS_INJECTION_KEY } from './tabsContext'

defineProps<{ value: string }>()
const context = inject(TABS_INJECTION_KEY)
if (!context) throw new Error('TabsContent must be used inside Tabs')
</script>

<template>
  <div v-if="context.active.value === value" role="tabpanel">
    <slot />
  </div>
</template>
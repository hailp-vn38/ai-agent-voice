<script setup lang="ts">
import { provide } from 'vue'

import { TABS_INJECTION_KEY } from './tabsContext'

const active = defineModel<string>({ required: true })

provide(TABS_INJECTION_KEY, { active })
</script>

<template>
  <div>
    <slot />
  </div>
</template>
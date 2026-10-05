<script setup lang="ts">
import { Moon, Sun } from '@lucide/vue'

import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useTheme } from '@/composables/useTheme'

const { dark, toggleTheme } = useTheme()
const { t } = useI18n()
</script>

<template>
  <Button variant="ghost" size="icon" :aria-label="t('app.themeToggle')" @click="toggleTheme">
    <Sun v-if="dark" class="size-4" />
    <Moon v-else class="size-4" />
  </Button>
</template>

<script setup lang="ts">
import { inject } from 'vue'

import { ACTION_MENU_INJECTION_KEY, type ActionMenuContext } from './context'

withDefaults(defineProps<{ variant?: 'default' | 'danger'; disabled?: boolean }>(), {
  variant: 'default',
  disabled: false,
})

const emit = defineEmits<{ select: [] }>()

const menu = inject<ActionMenuContext | undefined>(ACTION_MENU_INJECTION_KEY, undefined)

function onClick(event: MouseEvent) {
  if ((event.currentTarget as HTMLButtonElement).disabled) return
  emit('select')
  menu?.close({ restore: true })
  event.stopPropagation()
}
</script>

<template>
  <button
    type="button"
    role="menuitem"
    class="admin-menu-item disabled:cursor-not-allowed disabled:opacity-50"
    :class="variant === 'danger' ? 'text-danger' : undefined"
    tabindex="-1"
    :disabled="disabled"
    :aria-disabled="disabled || undefined"
    @click="onClick"
  >
    <slot />
  </button>
</template>
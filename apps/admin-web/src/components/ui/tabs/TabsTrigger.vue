<script setup lang="ts">
import { cva, type VariantProps } from 'class-variance-authority'
import { inject } from 'vue'

import { TABS_INJECTION_KEY } from './tabsContext'

const tabsTriggerVariants = cva(
  'inline-flex items-center gap-2 whitespace-nowrap rounded-md px-3 py-1.5 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 data-[state=active]:bg-background data-[state=active]:text-foreground data-[state=active]:shadow-sm',
  {
    variants: {
      variant: {
        default: '',
        ghost: 'data-[state=active]:bg-transparent data-[state=active]:shadow-none',
      },
    },
    defaultVariants: {
      variant: 'default',
    },
  },
)

type TabsTriggerVariants = VariantProps<typeof tabsTriggerVariants>

const props = withDefaults(
  defineProps<{
    value: string
    variant?: TabsTriggerVariants['variant']
    disabled?: boolean
  }>(),
  {
    variant: 'default',
    disabled: false,
  },
)

const injected = inject(TABS_INJECTION_KEY)
if (!injected) throw new Error('TabsTrigger must be used inside Tabs')
const context = injected

function select() {
  if (props.disabled) return
  context.active.value = props.value
}
</script>

<template>
  <button
    type="button"
    role="tab"
    :disabled="disabled"
    :aria-selected="context.active.value === value"
    :data-state="context.active.value === value ? 'active' : 'inactive'"
    :class="tabsTriggerVariants({ variant })"
    @click="select"
  >
    <slot />
  </button>
</template>
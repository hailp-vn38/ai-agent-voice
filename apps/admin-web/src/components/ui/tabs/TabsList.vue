<script setup lang="ts">
import { cva, type VariantProps } from 'class-variance-authority'

const tabsListVariants = cva('inline-flex items-center justify-start gap-1 overflow-x-auto rounded-lg bg-muted p-1 text-muted-foreground', {
  variants: {
    variant: {
      default: '',
      pill: 'rounded-full',
    },
  },
  defaultVariants: {
    variant: 'default',
  },
})

type TabsListVariants = VariantProps<typeof tabsListVariants>

defineProps<{
  variant?: TabsListVariants['variant']
}>()
</script>

<template>
  <div role="tablist" :class="tabsListVariants({ variant })">
    <slot />
  </div>
</template>
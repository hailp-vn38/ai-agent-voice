<script setup lang="ts">
import { ArrowLeft } from '@lucide/vue'

import { Button } from '@/components/ui/button'

defineProps<{
  title: string
  backLabel?: string
}>()

const emit = defineEmits<{ back: [] }>()
</script>

<template>
  <header class="studio-panel min-w-0 px-3 py-3 sm:px-4 sm:py-3.5" data-detail-header>
    <div class="flex min-w-0 flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
      <div class="flex min-w-0 items-center gap-2.5 sm:gap-3">
        <Button
          v-if="backLabel"
          size="sm"
          variant="ghost"
          class="h-9 shrink-0 cursor-pointer px-2 text-muted-foreground hover:text-foreground sm:px-2.5"
          :aria-label="backLabel"
          data-detail-back
          @click="emit('back')"
        >
          <ArrowLeft class="size-4" aria-hidden="true" />
          <span class="hidden sm:inline">{{ backLabel }}</span>
        </Button>

        <span v-if="backLabel" class="h-9 w-px shrink-0 bg-border/80" aria-hidden="true" />

        <slot name="icon" />

        <div class="min-w-0 flex-1">
          <h1 class="line-clamp-2 break-words text-xl font-semibold leading-tight tracking-tight sm:text-2xl" :title="title">
            {{ title }}
          </h1>
          <div v-if="$slots.details" class="mt-1 min-w-0 text-xs text-muted-foreground">
            <slot name="details" />
          </div>
        </div>
      </div>

      <div v-if="$slots.actions" class="flex shrink-0 flex-wrap items-center gap-2 pl-0 lg:pl-2" data-detail-actions>
        <slot name="actions" />
      </div>
    </div>
  </header>
</template>

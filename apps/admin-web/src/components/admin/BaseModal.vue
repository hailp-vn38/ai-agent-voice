<script setup lang="ts">
import { X } from '@lucide/vue'
import { nextTick, onBeforeUnmount, onMounted, ref, useId, watch } from 'vue'

import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const { t } = useI18n()

const props = withDefaults(defineProps<{
  title: string
  description?: string
  widthClass?: string
}>(), {
  description: '',
  widthClass: 'max-w-2xl',
})

const open = defineModel<boolean>({ required: true })

const panel = ref<HTMLElement>()
const titleId = useId()
const descriptionId = useId()
let restoreFocusTo: HTMLElement | null = null

function close() {
  open.value = false
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && open.value) close()
}

watch(open, async (value) => {
  if (value) {
    restoreFocusTo = (document.activeElement as HTMLElement | null) ?? null
    await nextTick()
    panel.value?.focus()
  } else {
    restoreFocusTo?.focus()
    restoreFocusTo = null
  }
})

onMounted(() => window.addEventListener('keydown', onKeydown))
onBeforeUnmount(() => window.removeEventListener('keydown', onKeydown))
</script>

<template>
  <Teleport to="body">
    <div v-if="open" class="fixed inset-0 z-50 flex items-center justify-center p-4 sm:p-6">
      <button class="absolute inset-0 bg-black/55 backdrop-blur-[2px]" :aria-label="t('common.closeDialog')" @click="close" />
      <section
        ref="panel"
        role="dialog"
        aria-modal="true"
        :aria-labelledby="titleId"
        :aria-describedby="props.description ? descriptionId : undefined"
        tabindex="-1"
        :class="['relative z-10 flex max-h-[90vh] w-full flex-col overflow-hidden rounded-xl border bg-background shadow-2xl outline-none', props.widthClass]"
      >
        <header class="flex items-start justify-between gap-4 border-b px-5 py-4 sm:px-6">
          <div class="min-w-0">
            <h2 :id="titleId" class="text-lg font-semibold tracking-tight">{{ title }}</h2>
            <p v-if="description" :id="descriptionId" class="mt-1 text-sm text-muted-foreground">{{ description }}</p>
          </div>
          <Button variant="ghost" size="icon" :aria-label="t('common.closeDialog')" @click="close">
            <X class="size-4" />
          </Button>
        </header>
        <div class="overflow-y-auto px-5 py-5 sm:px-6">
          <slot />
        </div>
        <footer v-if="$slots.footer" class="border-t px-5 py-4 sm:px-6">
          <slot name="footer" />
        </footer>
      </section>
    </div>
  </Teleport>
</template>

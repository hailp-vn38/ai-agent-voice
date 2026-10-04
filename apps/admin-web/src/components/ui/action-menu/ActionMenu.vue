<script setup lang="ts">
import { Ellipsis } from '@lucide/vue'
import { nextTick, onBeforeUnmount, provide, ref, useId, watch } from 'vue'

import { Button } from '@/components/ui/button'
import { ACTION_MENU_INJECTION_KEY } from './context'

const props = withDefaults(
  defineProps<{
    /** Accessible name for the trigger; also labels the menu itself. */
    label: string
    align?: 'start' | 'end'
    variant?: 'default' | 'ghost' | 'outline'
    size?: 'sm' | 'icon'
    panelWidth?: string
    disabled?: boolean
  }>(),
  {
    align: 'end',
    variant: 'ghost',
    size: 'icon',
    panelWidth: '13rem',
    disabled: false,
  },
)

const open = defineModel<boolean>('open', { default: false })

const anchor = ref<HTMLElement>()
const panel = ref<HTMLElement>()
const position = ref<{ top: string; left: string; maxHeight: string }>({
  top: '0px',
  left: '0px',
  maxHeight: '18rem',
})

const GUTTER = 8

const menuId = useId()
let restoreFocusTo: HTMLElement | null = null

function triggerEl() {
  return anchor.value?.querySelector('button') ?? undefined
}

function items() {
  if (!panel.value) return [] as HTMLElement[]
  return Array.from(panel.value.querySelectorAll<HTMLElement>('[role="menuitem"]:not([disabled])'))
}

function place() {
  const trigger = triggerEl()
  const surface = panel.value
  if (!trigger || !surface) return

  const rect = trigger.getBoundingClientRect()
  const width = surface.offsetWidth
  const height = surface.offsetHeight
  const offset = 6

  const spaceBelow = window.innerHeight - rect.bottom - offset
  const openUp = spaceBelow < height && rect.top > height
  const maxHeight = Math.max(120, Math.min(320, openUp ? rect.top - offset - GUTTER : spaceBelow - GUTTER))
  const top = openUp ? rect.top - offset - Math.min(height, maxHeight) : rect.bottom + offset
  const naturalLeft = props.align === 'end' ? rect.right - width : rect.left

  position.value = {
    top: `${Math.round(top)}px`,
    left: `${Math.round(Math.min(Math.max(GUTTER, naturalLeft), window.innerWidth - width - GUTTER))}px`,
    maxHeight: `${maxHeight}px`,
  }
}

async function openMenu() {
  if (props.disabled || open.value) return
  restoreFocusTo = (document.activeElement as HTMLElement | null) ?? null
  open.value = true
  await nextTick()
  place()
  items()[0]?.focus()
}

function close({ restore = true } = {}) {
  if (!open.value) return
  open.value = false
  if (restore) (restoreFocusTo ?? triggerEl())?.focus()
  restoreFocusTo = null
}

function toggle() {
  if (open.value) close()
  else void openMenu()
}

function onTriggerKeydown(event: KeyboardEvent) {
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault()
    void openMenu()
  }
}

function onPanelKeydown(event: KeyboardEvent) {
  const list = items()
  if (!list.length) return
  const index = list.indexOf(document.activeElement as HTMLElement)

  if (event.key === 'Escape') {
    event.preventDefault()
    close()
  } else if (event.key === 'ArrowDown') {
    event.preventDefault()
    list[(index + 1 + list.length) % list.length]?.focus()
  } else if (event.key === 'ArrowUp') {
    event.preventDefault()
    list[(index - 1 + list.length) % list.length]?.focus()
  } else if (event.key === 'Home') {
    event.preventDefault()
    list[0]?.focus()
  } else if (event.key === 'End') {
    event.preventDefault()
    list[list.length - 1]?.focus()
  } else if (event.key === 'Tab') {
    close({ restore: false })
  }
}

function onDocumentPointerdown(event: PointerEvent) {
  if (!open.value) return
  const target = event.target as Node
  if (panel.value?.contains(target) || anchor.value?.contains(target)) return
  close({ restore: false })
}

watch(open, (value) => {
  if (!value) return
  document.addEventListener('pointerdown', onDocumentPointerdown, true)
  window.addEventListener('resize', place)
  window.addEventListener('scroll', place, true)
})

provide(ACTION_MENU_INJECTION_KEY, { close })

onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', onDocumentPointerdown, true)
  window.removeEventListener('resize', place)
  window.removeEventListener('scroll', place, true)
})

defineExpose({ close })
</script>

<template>
  <span ref="anchor" class="inline-flex" @click.stop>
    <Button
      :variant="variant"
      :size="size"
      :disabled="disabled"
      :aria-label="label"
      aria-haspopup="menu"
      :aria-expanded="open"
      :aria-controls="open ? menuId : undefined"
      @click="toggle"
      @keydown="onTriggerKeydown"
    >
      <slot name="trigger">
        <Ellipsis v-if="size === 'icon'" class="size-4" />
      </slot>
    </Button>

    <Teleport to="body">
      <Transition
        enter-active-class="transition-opacity duration-150 ease-out"
        enter-from-class="opacity-0"
        leave-active-class="transition-opacity duration-100 ease-in"
        leave-to-class="opacity-0"
      >
        <div
          v-if="open"
          :id="menuId"
          ref="panel"
          role="menu"
          :aria-label="label"
          class="fixed z-50 overflow-x-hidden overflow-y-auto rounded-lg border bg-card p-1 text-card-foreground shadow-xl"
          :style="[position, { width: panelWidth, maxWidth: `calc(100vw - ${GUTTER * 2}px)` }]"
          @keydown="onPanelKeydown"
        >
          <slot :close="close" />
        </div>
      </Transition>
    </Teleport>
  </span>
</template>

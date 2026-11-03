<script lang="ts">
export type RightTab = 'outline' | 'chat'
</script>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref } from 'vue'
import { ListTree, Sparkles } from '@lucide/vue'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{ active: RightTab; order: RightTab[] }>()
const emit = defineEmits<{ select: [tab: RightTab]; reorder: [order: RightTab[]] }>()
const { t } = useI18n()

// A mouse starts dragging as soon as it moves; touch must hold first so a swipe still scrolls.
const MOUSE_HOLD_MS = 200
const TOUCH_HOLD_MS = 250
const DRAG_START_PX = 4
const SETTLE_MS = 160

type Slot = { left: number; right: number }
type Drag = { tab: RightTab; startX: number; dx: number; slots: Slot[]; bounds: Slot; gap: number }

const list = ref<HTMLElement | null>(null)
const drag = ref<Drag | null>(null)
// The dropped tab slides into its slot before the new order is committed.
const settling = ref(false)
// Turns transitions off for the frame where the committed order replaces the shifted tabs.
const instant = ref(false)
let pending: { tab: RightTab; pointerId: number; startX: number; startY: number; touch: boolean; target: HTMLElement } | null = null
let holdTimer: ReturnType<typeof setTimeout> | null = null
let settleTimer: ReturnType<typeof setTimeout> | null = null
// A release that ends a drag must not also select the tab under it.
let suppressClick = false

function clearHold() {
  if (holdTimer) clearTimeout(holdTimer)
  holdTimer = null
}
onBeforeUnmount(() => {
  clearHold()
  if (settleTimer) clearTimeout(settleTimer)
})

const fromIndex = computed(() => drag.value ? props.order.indexOf(drag.value.tab) : -1)
const toIndex = computed(() => {
  const current = drag.value
  if (!current) return -1
  const from = fromIndex.value
  const slot = current.slots[from]!
  // A tab gives way once the leading edge of the dragged tab passes its centre.
  const left = slot.left + current.dx
  const right = slot.right + current.dx
  return current.slots.filter((other, index) => {
    const centre = (other.left + other.right) / 2
    return index < from ? centre <= left : index > from && centre < right
  }).length
})

function beginDrag() {
  clearHold()
  const start = pending
  if (!start || !list.value) return
  const buttons = Array.from(list.value.querySelectorAll<HTMLElement>('.a-right-tab'))
  const slots = buttons.map(button => { const rect = button.getBoundingClientRect(); return { left: rect.left, right: rect.left + rect.width } })
  const listRect = list.value.getBoundingClientRect()
  const gap = Number.parseFloat(getComputedStyle(list.value).columnGap) || 0
  drag.value = { tab: start.tab, startX: start.startX, dx: 0, slots, bounds: { left: listRect.left, right: listRect.left + listRect.width }, gap }
  try { start.target.setPointerCapture(start.pointerId) } catch { /* pointer already released */ }
  if (start.touch) navigator.vibrate?.(8)
}

function onPointerDown(event: PointerEvent, tab: RightTab) {
  if (event.button !== 0 || settling.value) return
  const touch = event.pointerType === 'touch' || event.pointerType === 'pen'
  pending = { tab, pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, touch, target: event.currentTarget as HTMLElement }
  clearHold()
  holdTimer = setTimeout(beginDrag, touch ? TOUCH_HOLD_MS : MOUSE_HOLD_MS)
}

function onPointerMove(event: PointerEvent) {
  const current = drag.value
  if (!current) {
    if (!pending) return
    const moved = Math.hypot(event.clientX - pending.startX, event.clientY - pending.startY)
    if (moved <= DRAG_START_PX) return
    // A touch that moves before the hold is a scroll; a mouse that moves is a drag.
    if (pending.touch) { clearHold(); pending = null; return }
    beginDrag()
    if (!drag.value) return
    return onPointerMove(event)
  }
  event.preventDefault()
  const slot = current.slots[fromIndex.value]!
  // Keep the tab inside the tab bar.
  const min = current.bounds.left - slot.left
  const max = current.bounds.right - slot.right
  current.dx = Math.min(max, Math.max(min, event.clientX - current.startX))
}

function finishDrag() {
  clearHold()
  pending = null
  const current = drag.value
  if (!current || settling.value) return
  const from = fromIndex.value
  const to = toIndex.value
  // Holding still and letting go is still a click.
  suppressClick = to !== from || Math.abs(current.dx) > DRAG_START_PX
  const slot = current.slots[from]!
  const target = current.slots[to]!
  current.dx = to > from ? target.right - slot.right : to < from ? target.left - slot.left : 0
  settling.value = true
  settleTimer = setTimeout(() => {
    settleTimer = null
    const next = [...props.order]
    next.splice(from, 1)
    next.splice(to, 0, current.tab)
    instant.value = true
    drag.value = null
    settling.value = false
    if (to !== from) emit('reorder', next)
    void nextTick(() => requestAnimationFrame(() => { instant.value = false }))
  }, SETTLE_MS)
}

function tabStyle(tab: RightTab, index: number) {
  const current = drag.value
  if (!current) return undefined
  if (tab === current.tab) return { transform: `translate3d(${current.dx}px, 0, 0)` }
  const from = fromIndex.value
  const to = toIndex.value
  const slot = current.slots[from]!
  const shift = slot.right - slot.left + current.gap
  if (from < to && index > from && index <= to) return { transform: `translate3d(${-shift}px, 0, 0)` }
  if (to < from && index >= to && index < from) return { transform: `translate3d(${shift}px, 0, 0)` }
  return undefined
}

function onClick(tab: RightTab) {
  if (suppressClick) { suppressClick = false; return }
  if (tab !== props.active) emit('select', tab)
}

function moveWithKeyboard(tab: RightTab, step: -1 | 1) {
  const index = props.order.indexOf(tab)
  const target = index + step
  if (target < 0 || target >= props.order.length) return
  const next = [...props.order]
  next.splice(index, 1)
  next.splice(target, 0, tab)
  emit('reorder', next)
}
</script>

<template>
  <div ref="list" class="a-right-tabs" role="tablist" :aria-label="t('knowledgeV3.rightPanel')"
    :class="{ 'is-reordering': drag, 'is-settling': settling, 'is-instant': instant }">
    <button v-for="(tab, index) in order" :key="tab" type="button" role="tab" class="a-right-tab"
      :class="{ active: tab === active, 'is-dragging': tab === drag?.tab }" :data-tab="tab" :style="tabStyle(tab, index)"
      :aria-selected="tab === active ? 'true' : 'false'" :title="t('knowledgeV3.rightTabReorderHint')"
      @pointerdown="onPointerDown($event, tab)" @pointermove="onPointerMove"
      @pointerup="finishDrag" @pointercancel="finishDrag" @lostpointercapture="finishDrag"
      @contextmenu="(drag || pending) && $event.preventDefault()" @click="onClick(tab)"
      @keydown.alt.left.prevent="moveWithKeyboard(tab, -1)" @keydown.alt.right.prevent="moveWithKeyboard(tab, 1)">
      <template v-if="tab === 'outline'"><ListTree :size="13" aria-hidden="true" /> {{ t('knowledgeV3.rightOutline') }}</template>
      <template v-else><Sparkles :size="13" aria-hidden="true" /> {{ t('knowledgeV3.chatTitle') }}</template>
    </button>
  </div>
</template>

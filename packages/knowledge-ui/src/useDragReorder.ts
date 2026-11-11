import { computed, nextTick, onBeforeUnmount, ref, type Ref } from 'vue'

// A mouse starts dragging as soon as it moves; touch must hold first so a swipe still scrolls.
const MOUSE_HOLD_MS = 200
const TOUCH_HOLD_MS = 250
const DRAG_START_PX = 4
const SETTLE_MS = 160
// Holding the item this close to an edge of a scrollable row scrolls it, faster nearer the edge.
const EDGE_SCROLL_PX = 40
const MAX_SCROLL_STEP_PX = 16

type Slot = { left: number; right: number }
/** Positions are viewport coordinates from the moment the drag began; scrolling since then is added to `dx`. */
type Drag<K> = {
  key: K; from: number; startX: number; pointerX: number; dx: number; slots: Slot[]; bounds: Slot; gap: number
  container: HTMLElement; startScroll: number
}
type Pending<K> = { key: K; pointerId: number; startX: number; startY: number; touch: boolean; target: HTMLElement }

export type DragReorderOptions<K extends string> = {
  /** The element that holds the items and bounds the drag. */
  container: Ref<HTMLElement | null>
  /** The draggable items in their current order. Each is rendered with `data-reorder-key` set to its key. */
  order: () => K[]
  onReorder: (order: K[]) => void
  /** Presses that start inside a matching element, such as a close button, never drag. */
  ignore?: string
}

/**
 * Press-and-hold drag reordering for one row of items. The held item follows the pointer,
 * the items it passes slide aside, and the new order is reported once the item settles.
 */
export function useDragReorder<K extends string>(options: DragReorderOptions<K>) {
  const drag = ref<Drag<K> | null>(null) as Ref<Drag<K> | null>
  // The dropped item slides into its slot before the new order is committed.
  const settling = ref(false)
  // Turns transitions off for the frame where the committed order replaces the shifted items.
  const instant = ref(false)
  let pending: Pending<K> | null = null
  let holdTimer: ReturnType<typeof setTimeout> | null = null
  let settleTimer: ReturnType<typeof setTimeout> | null = null
  // A release that ends a drag must not also activate the item under it.
  let suppressClick = false
  let scrollFrame: number | null = null

  function clearHold() {
    if (holdTimer) clearTimeout(holdTimer)
    holdTimer = null
  }
  // A touch drag must keep the list from scrolling, which touch-action cannot change mid-gesture.
  function blockTouchScroll(event: TouchEvent) {
    if (drag.value) event.preventDefault()
  }
  function stopBlockingTouchScroll() {
    window.removeEventListener('touchmove', blockTouchScroll)
  }
  onBeforeUnmount(() => {
    clearHold()
    if (settleTimer) clearTimeout(settleTimer)
    stopBlockingTouchScroll()
    stopEdgeScroll()
  })

  /** Places the held item under the pointer, counting any scrolling of the row since the drag began. */
  function follow() {
    const current = drag.value
    if (!current) return
    const scrolled = current.container.scrollLeft - current.startScroll
    const slot = current.slots[current.from]!
    // Keep the item inside the visible part of its row.
    const min = current.bounds.left + scrolled - slot.left
    const max = current.bounds.right + scrolled - slot.right
    current.dx = Math.min(Math.max(max, 0), Math.max(Math.min(min, 0), current.pointerX - current.startX + scrolled))
  }
  /** One step of edge scrolling; returns whether the row moved. */
  function edgeScrollStep(): boolean {
    const current = drag.value
    if (!current || settling.value) return false
    const { container, bounds, pointerX } = current
    const edge = Math.min(EDGE_SCROLL_PX, (bounds.right - bounds.left) / 4)
    let step = 0
    if (pointerX < bounds.left + edge) step = -Math.min(1.5, (bounds.left + edge - pointerX) / edge)
    else if (pointerX > bounds.right - edge) step = Math.min(1.5, (pointerX - (bounds.right - edge)) / edge)
    if (!step) return false
    const before = container.scrollLeft
    container.scrollLeft = before + Math.round(step * MAX_SCROLL_STEP_PX) || before + Math.sign(step)
    if (container.scrollLeft === before) return false
    follow()
    return true
  }
  function edgeScrollLoop() {
    scrollFrame = null
    if (!drag.value || settling.value) return
    edgeScrollStep()
    scrollFrame = requestAnimationFrame(edgeScrollLoop)
  }
  function stopEdgeScroll() {
    if (scrollFrame !== null) cancelAnimationFrame(scrollFrame)
    scrollFrame = null
    drag.value?.container.removeEventListener('scroll', follow)
  }

  const toIndex = computed(() => {
    const current = drag.value
    if (!current) return -1
    const slot = current.slots[current.from]!
    // An item gives way once the leading edge of the dragged item passes its centre.
    const left = slot.left + current.dx
    const right = slot.right + current.dx
    return current.slots.filter((other, index) => {
      const centre = (other.left + other.right) / 2
      return index < current.from ? centre <= left : index > current.from && centre < right
    }).length
  })

  function beginDrag() {
    clearHold()
    const start = pending
    const container = options.container.value
    if (!start || !container) return
    const order = options.order()
    const from = order.indexOf(start.key)
    if (from < 0) return
    const elements = new Map(Array.from(container.querySelectorAll<HTMLElement>('[data-reorder-key]'))
      .map(element => [element.dataset.reorderKey, element]))
    const slots = order.map(key => {
      const rect = elements.get(key)?.getBoundingClientRect()
      return rect ? { left: rect.left, right: rect.left + rect.width } : { left: 0, right: 0 }
    })
    const box = container.getBoundingClientRect()
    // A pinned item, such as a sticky first tab, covers the start of the row.
    const pinned = Array.from(container.querySelectorAll<HTMLElement>('[data-reorder-pinned]'))
      .reduce((edge, element) => { const rect = element.getBoundingClientRect(); return Math.max(edge, rect.left + rect.width) }, box.left)
    const gap = Number.parseFloat(getComputedStyle(container).columnGap) || 0
    drag.value = {
      key: start.key, from, startX: start.startX, pointerX: start.startX, dx: 0, slots,
      bounds: { left: pinned, right: box.left + box.width }, gap, container, startScroll: container.scrollLeft,
    }
    // A wheel or trackpad scroll during the drag keeps the item under the pointer.
    container.addEventListener('scroll', follow)
    scrollFrame = requestAnimationFrame(edgeScrollLoop)
    try { start.target.setPointerCapture(start.pointerId) } catch { /* pointer already released */ }
    if (start.touch) {
      window.addEventListener('touchmove', blockTouchScroll, { passive: false })
      navigator.vibrate?.(8)
    }
  }

  function onPointerDown(event: PointerEvent, key: K) {
    if (event.button !== 0 || settling.value) return
    if (options.ignore && (event.target as Element | null)?.closest?.(options.ignore)) return
    const touch = event.pointerType === 'touch' || event.pointerType === 'pen'
    pending = { key, pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, touch, target: event.currentTarget as HTMLElement }
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
      if (drag.value) onPointerMove(event)
      return
    }
    event.preventDefault()
    current.pointerX = event.clientX
    follow()
  }

  function finishDrag() {
    clearHold()
    pending = null
    stopBlockingTouchScroll()
    const current = drag.value
    if (!current || settling.value) return
    stopEdgeScroll()
    const from = current.from
    const to = toIndex.value
    // Holding still and letting go is still a click.
    suppressClick = to !== from || Math.abs(current.dx) > DRAG_START_PX
    const slot = current.slots[from]!
    const target = current.slots[to]!
    current.dx = to > from ? target.right - slot.right : to < from ? target.left - slot.left : 0
    settling.value = true
    settleTimer = setTimeout(() => {
      settleTimer = null
      const next = [...options.order()]
      next.splice(from, 1)
      next.splice(to, 0, current.key)
      instant.value = true
      drag.value = null
      settling.value = false
      if (to !== from) options.onReorder(next)
      void nextTick(() => requestAnimationFrame(() => { instant.value = false }))
    }, SETTLE_MS)
  }

  /** True when this click ends a drag and must be ignored. */
  function consumeClick(): boolean {
    if (!suppressClick) return false
    suppressClick = false
    return true
  }

  function itemStyle(key: K) {
    const current = drag.value
    if (!current) return undefined
    if (key === current.key) return { transform: `translate3d(${current.dx}px, 0, 0)` }
    const index = options.order().indexOf(key)
    const from = current.from
    const to = toIndex.value
    const slot = current.slots[from]!
    const shift = slot.right - slot.left + current.gap
    if (from < to && index > from && index <= to) return { transform: `translate3d(${-shift}px, 0, 0)` }
    if (to < from && index >= to && index < from) return { transform: `translate3d(${shift}px, 0, 0)` }
    return undefined
  }

  const draggingKey = computed(() => drag.value?.key ?? null)
  const listeners = (key: K) => ({
    pointerdown: (event: PointerEvent) => onPointerDown(event, key),
    pointermove: onPointerMove,
    pointerup: finishDrag,
    pointercancel: finishDrag,
    lostpointercapture: finishDrag,
    contextmenu: (event: Event) => { if (drag.value || pending) event.preventDefault() },
  })

  return { draggingKey, settling, instant, itemStyle, listeners, consumeClick }
}

/** Moves `key` one place towards `step`, or returns null at either end. */
export function movedOrder<K>(order: K[], key: K, step: -1 | 1): K[] | null {
  const index = order.indexOf(key)
  const target = index + step
  if (index < 0 || target < 0 || target >= order.length) return null
  const next = [...order]
  next.splice(index, 1)
  next.splice(target, 0, key)
  return next
}

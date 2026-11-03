import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { defineComponent, h, ref } from 'vue'
import { useDragReorder } from './useDragReorder'

beforeEach(() => { vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'requestAnimationFrame', 'cancelAnimationFrame'] }) })
afterEach(() => { vi.useRealTimers() })

// Three 100px items in a row that shows only the first two.
const Row = defineComponent({
  emits: ['reorder'],
  setup(_, { emit }) {
    const list = ref<HTMLElement | null>(null)
    const order = ref(['a', 'b', 'c'])
    const reorder = useDragReorder<string>({
      container: list, order: () => order.value,
      onReorder: next => { order.value = next; emit('reorder', next) },
    })
    // A render function takes `onPointerdown` where a template's v-on takes `pointerdown`.
    const handlers = (key: string) => Object.fromEntries(Object.entries(reorder.listeners(key))
      .map(([name, handler]) => ['on' + name[0]!.toUpperCase() + name.slice(1), handler]))
    return () => h('div', { ref: list }, order.value.map(key =>
      h('button', { key, 'data-reorder-key': key, style: reorder.itemStyle(key), ...handlers(key) }, key)))
  },
})

function mountRow() {
  const wrapper = mount(Row)
  const list = wrapper.element as HTMLElement
  list.getBoundingClientRect = () => ({ left: 0, width: 200, top: 0, height: 30 } as DOMRect)
  wrapper.findAll('button').forEach((item, index) => {
    // Layout positions move left as the row scrolls.
    item.element.getBoundingClientRect = () => ({ left: index * 100 - list.scrollLeft, width: 100, top: 0, height: 30 } as DOMRect)
    ;(item.element as HTMLElement).setPointerCapture = () => {}
  })
  return { wrapper, list }
}

function pointer(element: Element, type: string, clientX: number) {
  element.dispatchEvent(new MouseEvent(type, { bubbles: true, button: 0, clientX, clientY: 10 }))
}

it('scrolls the row while the item is held at its edge and drops it past the hidden items', async () => {
  const { wrapper, list } = mountRow()
  const a = wrapper.get('[data-reorder-key="a"]').element
  pointer(a, 'pointerdown', 50)
  pointer(a, 'pointermove', 195)
  vi.advanceTimersByTime(200)
  expect(list.scrollLeft).toBeGreaterThanOrEqual(100)
  pointer(a, 'pointerup', 195)
  const scrolled = list.scrollLeft
  vi.advanceTimersByTime(400)
  expect(list.scrollLeft).toBe(scrolled)
  expect(wrapper.emitted('reorder')).toEqual([[['b', 'c', 'a']]])
})

it('keeps the held item under the pointer when the row is scrolled by the wheel', async () => {
  const { wrapper, list } = mountRow()
  const a = wrapper.get('[data-reorder-key="a"]').element
  pointer(a, 'pointerdown', 50)
  pointer(a, 'pointermove', 100)
  list.scrollLeft = 30
  list.dispatchEvent(new Event('scroll'))
  await wrapper.vm.$nextTick()
  expect(a.getAttribute('style')).toContain('translate3d(80px')
})

import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import RightPanelTabs from '@vela/knowledge-ui/components/RightPanelTabs.vue'

beforeEach(() => { vi.useFakeTimers() })
afterEach(() => { vi.useRealTimers() })

// jsdom has no PointerEvent, so a MouseEvent carries the coordinates under the pointer event's name.
async function pointer(element: Element, type: string, clientX = 0, pointerType = 'mouse') {
  const event = new MouseEvent(type, { bubbles: true, button: 0, clientX, clientY: 10 })
  Object.defineProperty(event, 'pointerType', { value: pointerType })
  element.dispatchEvent(event)
  await Promise.resolve()
}

function mountTabs(active: 'outline' | 'chat' = 'outline') {
  const wrapper = mount(RightPanelTabs, { props: { active, order: ['outline', 'chat'] } })
  wrapper.element.getBoundingClientRect = () => ({ left: 0, width: 202, top: 0, height: 26 } as DOMRect)
  wrapper.findAll('.a-right-tab').forEach((tab, index) => {
    tab.element.getBoundingClientRect = () => ({ left: index * 102, width: 100, top: 0, height: 26 } as DOMRect)
    ;(tab.element as HTMLElement).setPointerCapture = () => {}
  })
  return wrapper
}

it('drags a tab with the mouse, slides the other aside and commits the order once it settles', async () => {
  const wrapper = mountTabs()
  const outline = wrapper.get('[data-tab="outline"]')
  await pointer(outline.element, 'pointerdown', 50)
  await pointer(outline.element, 'pointermove', 130)
  await wrapper.vm.$nextTick()
  expect(outline.classes()).toContain('is-dragging')
  expect(outline.attributes('style')).toContain('translate3d(80px')
  expect(wrapper.get('[data-tab="chat"]').attributes('style')).toContain('translate3d(-100px')

  await pointer(outline.element, 'pointerup', 130)
  await outline.trigger('click')
  expect(wrapper.emitted('reorder')).toBeUndefined()
  vi.advanceTimersByTime(200)
  expect(wrapper.emitted('reorder')).toEqual([[['chat', 'outline']]])
  expect(wrapper.emitted('select')).toBeUndefined()
})

it('keeps a touch that moves before the hold as a scroll, and a tap as a selection', async () => {
  const wrapper = mountTabs()
  const chat = wrapper.get('[data-tab="chat"]')
  await pointer(chat.element, 'pointerdown', 150, 'touch')
  await pointer(chat.element, 'pointermove', 20, 'touch')
  vi.advanceTimersByTime(400)
  await pointer(chat.element, 'pointerup', 20, 'touch')
  await chat.trigger('click')
  vi.advanceTimersByTime(400)
  expect(wrapper.emitted('reorder')).toBeUndefined()
  expect(wrapper.emitted('select')).toEqual([['chat']])
})

it('drags a touch after a press and hold', async () => {
  const wrapper = mountTabs()
  const chat = wrapper.get('[data-tab="chat"]')
  await pointer(chat.element, 'pointerdown', 150, 'touch')
  vi.advanceTimersByTime(300)
  await pointer(chat.element, 'pointermove', 40, 'touch')
  await pointer(chat.element, 'pointerup', 40, 'touch')
  vi.advanceTimersByTime(200)
  expect(wrapper.emitted('reorder')).toEqual([[['chat', 'outline']]])
})

it('moves the focused tab with Alt and an arrow key', async () => {
  const wrapper = mountTabs('chat')
  await wrapper.get('[data-tab="chat"]').trigger('keydown', { key: 'ArrowLeft', altKey: true })
  expect(wrapper.emitted('reorder')).toEqual([[['chat', 'outline']]])
})

it('hides the Chat tab when the backend does not support chat', () => {
  const wrapper = mount(RightPanelTabs, {
    props: { active: 'outline', order: ['outline', 'chat'], capabilities: { chat: false } },
  })
  expect(wrapper.findAll('[role="tab"]').map(tab => tab.attributes('data-tab'))).toEqual(['outline'])
})

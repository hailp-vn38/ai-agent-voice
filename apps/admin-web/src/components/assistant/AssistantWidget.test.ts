import { createPinia, setActivePinia } from 'pinia'
import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'
import AssistantWidget from './AssistantWidget.vue'
import { useAssistantStore } from '@/stores/assistant'
import { useAuthStore } from '@/stores/auth'
import { setLocale } from '@/composables/useI18n'

let widget: ReturnType<typeof mount>
let assistant: ReturnType<typeof useAssistantStore>
beforeEach(async () => {
  document.body.innerHTML = ''
  setLocale('en')
  const pinia = createPinia()
  setActivePinia(pinia)
  const auth = useAuthStore()
  auth.setAdminToken('test-token')
  assistant = useAssistantStore()
  assistant.visible = true
  assistant.state = 'ready'
  widget = mount(AssistantWidget, {
    global: { plugins: [pinia], stubs: { Teleport: true, Transition: false, RouterLink: true } },
  })
  await nextTick()
})
afterEach(() => {
  widget.unmount()
  document.body.innerHTML = ''
  sessionStorage.clear()
  vi.restoreAllMocks()
})

describe('compact Studio Assistant', () => {
  it('shows only the latest user/LLM turn and offers history on demand', async () => {
    const store = useAssistantStore()
    store.messages.push(
      { id: 1, role: 'user', text: 'older question', status: 'completed' },
      { id: 2, role: 'assistant', text: 'older answer', status: 'completed' },
      { id: 3, role: 'user', text: 'current ASR result', status: 'accepted' },
      { id: 4, role: 'assistant', text: 'current LLM answer', status: 'pending' },
    )
    await nextTick()
    expect(widget.text()).toContain('current ASR result')
    expect(widget.text()).toContain('current LLM answer')
    expect(widget.text()).not.toContain('older answer')
    const history = widget.get('button[aria-pressed="false"]')
    await history.trigger('click')
    expect(widget.text()).toContain('older question')
    expect(widget.text()).toContain('older answer')
  })

  it('collapses on outside click without disconnecting or clearing a speaking turn', async () => {
    const store = useAssistantStore()
    store.state = 'speaking'
    store.messages.push({ id: 1, role: 'assistant', text: 'still speaking', status: 'pending' })
    const disconnect = vi.spyOn(store, 'disconnect')
    document.body.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
    await nextTick()
    expect(store.visible).toBe(false)
    expect(store.state).toBe('speaking')
    expect(store.messages).toHaveLength(1)
    expect(disconnect).not.toHaveBeenCalled()
  })

  it('stops hidden microphone capture without aborting or disconnecting the session', async () => {
    const store = useAssistantStore()
    store.state = 'listening'
    store.micEnabled = true
    const stop = vi.spyOn(store, 'stopMicrophone').mockResolvedValue(undefined)
    const interrupt = vi.spyOn(store, 'interrupt').mockImplementation(() => {})
    const disconnect = vi.spyOn(store, 'disconnect')
    document.body.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
    await nextTick()
    expect(store.visible).toBe(false)
    expect(stop).toHaveBeenCalledOnce()
    expect(interrupt).not.toHaveBeenCalled()
    expect(disconnect).not.toHaveBeenCalled()
  })

  it('keeps the microphone stop control available while the assistant speaks', async () => {
    const store = useAssistantStore()
    store.micEnabled = true
    store.state = 'speaking'
    const stop = vi.spyOn(store, 'stopMicrophone').mockResolvedValue(undefined)
    await nextTick()
    const button = widget.get('button[aria-label="Stop recording"]')
    expect(button.attributes('aria-pressed')).toBe('true')
    expect(button.attributes('disabled')).toBeUndefined()
    await button.trigger('click')
    expect(stop).toHaveBeenCalledOnce()
  })

  it('shows active tool below waveform without adding conversation bubbles', async () => {
    assistant.state = 'processing'
    assistant.activeTool = { turn_id: 3, name: 'external.home_assistant.light_turn_on' }
    await nextTick()
    expect(widget.text()).toContain('Using external.home_assistant.light_turn_on')
    expect(widget.findAll('article')).toHaveLength(0)
    assistant.activeTool = null
    await nextTick()
    expect(widget.text()).toContain('Assistant is preparing a response')
  })

  it('uses a connection dot, prioritizes microphone, and reveals text input only on demand', async () => {
    expect(widget.find('.assistant-connection-dot.is-online').exists()).toBe(true)
    expect(widget.get('header .sr-only').text()).toBe('Connected')
    expect(widget.text()).not.toContain('Use Agent default')
    expect(widget.find('textarea').exists()).toBe(false)
    expect(widget.find('button[aria-label="Microphone"]').exists()).toBe(true)
    await widget.get('button[aria-label="Switch to text input"]').trigger('click')
    expect(widget.find('textarea').exists()).toBe(true)
    await widget.get('button[aria-label="Switch to voice"]').trigger('click')
    expect(widget.find('textarea').exists()).toBe(false)
  })
})

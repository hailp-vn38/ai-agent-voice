import { defineStore } from 'pinia'
import { ref, shallowRef } from 'vue'
import type { AssistantSettings } from '@/api/assistant'
import { WebAssistantClient, type AssistantState } from '@/lib/assistant/voiceClient'

export interface AssistantMessage { id: number; role: 'user' | 'assistant'; text: string; status: 'pending' | 'accepted' | 'completed' | 'failed' | 'cancelled' | 'uncertain' }
export const useAssistantStore = defineStore('assistant', () => {
  const visible = ref(false)
  const state = ref<AssistantState>('disconnected')
  const error = ref('')
  const settings = shallowRef<AssistantSettings>()
  const messages = ref<AssistantMessage[]>([])
  const activeTool = ref<{ name: string; turn_id: number } | null>(null)
  const muted = ref(true)
  const level = ref(0)
  const micEnabled = ref(false)
  let id = 0
  let pendingUser: AssistantMessage | undefined
  let response: AssistantMessage | undefined
  let manuallyMuted = false
  function add(role: AssistantMessage['role'], text: string, status: AssistantMessage['status']) {
    const message: AssistantMessage = { id: ++id, role, text, status }
    messages.value.push(message)
    if (messages.value.length > 200) messages.value.shift()
    return messages.value[messages.value.length - 1]!
  }
  const client = new WebAssistantClient({
    level: value => { level.value = value },
    microphone: enabled => { micEnabled.value = enabled },
    state: value => {
      state.value = value
      if (value !== 'processing') activeTool.value = null
      if (value === 'uncertain') {
        if (pendingUser) pendingUser.status = 'uncertain'
        if (response) response.status = 'uncertain'
      }
      muted.value = client.audio.muted
    },
    settings: value => { settings.value = value },
    error: code => { error.value = code; muted.value = client.audio.muted },
    event: event => {
      if (event.type === 'stt') {
        if (pendingUser) { pendingUser.text = event.text; pendingUser.status = 'accepted' }
        else pendingUser = add('user', event.text, 'accepted')
      } else if (event.type === 'llm') {
        response ??= add('assistant', '', 'pending')
        if (response.text.length + event.text.length > 262144) { error.value = 'response_too_large'; client.disconnect(); return }
        response.text += event.text
      } else if (event.type === 'tool') {
        if (event.state === 'started' && state.value === 'processing') {
          activeTool.value = { name: event.name, turn_id: event.turn_id }
        } else if (event.state !== 'started' && activeTool.value?.turn_id === event.turn_id && activeTool.value.name === event.name) {
          activeTool.value = null
        }
      } else if (event.type === 'turn_status' && event.state !== 'listening_ready') {
        activeTool.value = null
        const status = event.state === 'rejected' ? 'failed' : event.state
        if (pendingUser) pendingUser.status = status
        if (response) response.status = status
        if (event.state === 'rejected' || event.state === 'failed') error.value = event.reason ?? 'turn_failed'
        pendingUser = undefined
        response = undefined
      }
    },
  })
  async function run(action: () => Promise<void>) {
    error.value = ''
    try { await action() } catch (e) {
      error.value = e instanceof Error && /^[a-z_]{1,64}$/.test(e.message) ? e.message : 'operation_failed'
    }
  }
  async function connect() { pendingUser = undefined; response = undefined; activeTool.value = null; await run(() => client.connect()) }
  async function open() { visible.value = true; if (['disconnected', 'disabled', 'unconfigured', 'error', 'uncertain'].includes(state.value)) await connect() }
  async function sendText(text: string) {
    if (state.value !== 'ready') return
    activeTool.value = null
    pendingUser = add('user', text.trim(), 'pending')
    response = undefined
    await run(() => client.sendText(text))
    if (pendingUser?.status === 'pending' && error.value) pendingUser.status = 'failed'
  }
  async function startMicrophone() {
    activeTool.value = null
    pendingUser = undefined
    response = undefined
    // Unlock TTS playback from the microphone click's user gesture; respect explicit mute.
    if (!manuallyMuted && client.audio.muted) {
      try { await client.audio.enableSpeaker(); muted.value = false } catch { /* Microphone and transcript remain usable without playback support. */ }
    }
    await run(() => client.startMicrophone())
  }
  async function toggleSpeaker() {
    await run(async () => {
      if (client.audio.muted) {
        await client.audio.enableSpeaker()
        if (state.value === 'speaking') client.audio.beginPlayback()
      } else client.audio.muteSpeaker()
      muted.value = client.audio.muted
      manuallyMuted = client.audio.muted
    })
  }
  function disconnect() {
    client.disconnect()
    activeTool.value = null
    messages.value = []
    pendingUser = response = undefined
    error.value = ''
    settings.value = undefined
    muted.value = true
    micEnabled.value = false
    manuallyMuted = false
  }
  return { visible, state, error, settings, messages, activeTool, muted, micEnabled, level, open, connect, sendText, startMicrophone, toggleSpeaker, disconnect,
    stopMicrophone: () => run(() => client.stopMicrophone()), interrupt: () => client.interrupt() }
})

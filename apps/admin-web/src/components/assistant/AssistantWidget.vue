<script setup lang="ts">
import { AudioLines, Bot, History, Keyboard, LoaderCircle, Mic, Send, Square, Volume2, VolumeX, X } from '@lucide/vue'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { AssistantAudio } from '@/lib/assistant/audio'
import { useAssistantStore } from '@/stores/assistant'
import { useAuthStore } from '@/stores/auth'

const assistant = useAssistantStore()
const auth = useAuthStore()
const { t } = useI18n()
const draft = ref('')
const textMode = ref(false)
const historyOpen = ref(false)
const launcher = ref<HTMLElement>()
const panel = ref<HTMLElement>()
const input = ref<HTMLTextAreaElement>()
const transcript = ref<HTMLElement>()
const micSupported = AssistantAudio.microphoneSupported()
const bars = Array.from({ length: 39 }, (_, index) => index)
const active = computed(() => ['listening', 'processing', 'speaking', 'aborting'].includes(assistant.state))
const connected = computed(() => ['ready', 'listening', 'processing', 'speaking', 'aborting'].includes(assistant.state))
const connecting = computed(() => ['connecting', 'handshaking', 'reconnecting'].includes(assistant.state))
const connectionTone = computed(() => connected.value ? 'online' : connecting.value ? 'connecting' : 'offline')
const voiceMode = computed(() => {
  if (assistant.state === 'listening') return 'listening'
  if (assistant.state === 'speaking') return 'speaking'
  if (['processing', 'aborting'].includes(assistant.state)) return 'thinking'
  return 'idle'
})
const latestMessages = computed(() => {
  const all = assistant.messages
  // One ASR/user message and its LLM response form the compact live turn.
  for (let index = all.length - 1; index >= 0; index--) {
    if (all[index]?.role === 'user') return all.slice(index)
  }
  return all.slice(-1)
})
const displayedMessages = computed(() => historyOpen.value ? assistant.messages : latestMessages.value)
const canReconnect = computed(() => ['unconfigured', 'disabled', 'error', 'uncertain', 'disconnected', 'auth_error'].includes(assistant.state))

function barHeight(index: number) {
  const shape = 0.22 + 0.78 * Math.abs(Math.sin((index + 2) * 1.07) * Math.cos(index * 0.47))
  if (voiceMode.value === 'listening') return Math.round(4 + Math.min(1, assistant.level * 6) * 43 * shape) + 'px'
  if (voiceMode.value === 'speaking') return Math.round(9 + shape * 30) + 'px'
  if (voiceMode.value === 'thinking') return Math.round(5 + shape * 13) + 'px'
  return Math.round(3 + shape * 4) + 'px'
}
function collapse(focusLauncher = true) {
  // Hide the panel without ending the WebSocket or interrupting TTS.
  // Microphone capture is stopped for privacy whenever the panel is hidden.
  if (assistant.micEnabled) void assistant.stopMicrophone()
  assistant.visible = false
  historyOpen.value = false
  if (focusLauncher) launcher.value?.focus()
}
function closeSession() {
  if (active.value && !window.confirm(t('assistant.endConfirm'))) return
  assistant.disconnect()
  draft.value = ''
  textMode.value = false
  collapse()
}
async function send() {
  const value = draft.value.trim()
  if (!value || assistant.state !== 'ready') return
  draft.value = ''
  await assistant.sendText(value)
}
async function microphone() {
  if (assistant.micEnabled) await assistant.stopMicrophone()
  else if (assistant.state === 'ready' && micSupported) await assistant.startMicrophone()
}
async function toggleTextMode() {
  textMode.value = !textMode.value
  if (textMode.value) {
    await nextTick()
    input.value?.focus()
  } else panel.value?.focus()
}
function outside(event: PointerEvent) {
  if (!assistant.visible || !(event.target instanceof Node)) return
  if (!panel.value?.contains(event.target) && !launcher.value?.contains(event.target)) collapse(false)
}
function keydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && assistant.visible) { event.preventDefault(); collapse() }
}
function visibility() {
  // Background microphone capture is not permitted; an ongoing TTS downlink is preserved.
  if (document.hidden && assistant.micEnabled) void assistant.stopMicrophone()
}
watch(() => auth.adminToken, () => { assistant.disconnect(); assistant.visible = false })
watch(() => assistant.visible, async open => {
  if (open) {
    await nextTick()
    panel.value?.focus()
  }
})
watch(() => [assistant.messages.map(message => message.text).join(''), historyOpen.value, assistant.visible], async () => {
  if (!assistant.visible) return
  await nextTick()
  if (transcript.value) transcript.value.scrollTop = transcript.value.scrollHeight
})
onMounted(() => {
  document.addEventListener('pointerdown', outside)
  document.addEventListener('visibilitychange', visibility)
  window.addEventListener('keydown', keydown)
})
onBeforeUnmount(() => {
  assistant.disconnect()
  document.removeEventListener('pointerdown', outside)
  document.removeEventListener('visibilitychange', visibility)
  window.removeEventListener('keydown', keydown)
})
</script>

<template>
  <Teleport to="body">
    <div v-if="auth.adminToken" class="text-foreground">
      <button
        ref="launcher" type="button"
        class="assistant-launcher fixed z-30 grid size-14 place-items-center rounded-full bg-primary text-primary-foreground shadow-lg focus-visible:outline-2 focus-visible:outline-offset-4"
        :class="{ 'assistant-launcher-active': active, 'assistant-launcher-listening': assistant.micEnabled }"
        :aria-label="assistant.visible ? t('assistant.collapse') : t('assistant.open')"
        :aria-expanded="assistant.visible" aria-controls="studio-assistant"
        @click="assistant.visible ? collapse() : assistant.open()"
      >
        <AudioLines v-if="active" class="size-6" aria-hidden="true" />
        <Bot v-else class="size-6" aria-hidden="true" />
        <span class="assistant-launcher-dot" :class="'is-' + connectionTone" aria-hidden="true" />
      </button>
      <Transition name="assistant-panel">
        <section
          v-if="assistant.visible" id="studio-assistant" ref="panel" tabindex="-1"
          class="assistant-panel fixed z-40 flex flex-col overflow-hidden border bg-card shadow-2xl"
          role="dialog" :aria-label="t('assistant.title')"
        >
          <header class="flex h-14 shrink-0 items-center gap-2 border-b px-3">
            <span class="grid size-9 shrink-0 place-items-center rounded-xl bg-primary/10 text-primary"><Bot class="size-5" aria-hidden="true" /></span>
            <div class="min-w-0 flex-1">
              <strong class="block truncate text-sm">{{ t('assistant.title') }}</strong>
              <span class="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                <span class="assistant-connection-dot" :class="'is-' + connectionTone" aria-hidden="true" />
                <span class="sr-only">{{ t(`assistant.state.${assistant.state}`) }}</span>
              </span>
            </div>
            <Button size="icon" variant="ghost" class="size-8 shrink-0 rounded-full" :aria-label="t('assistant.disconnect')" :title="t('assistant.disconnect')" @click="closeSession">
              <X class="size-4" aria-hidden="true" />
            </Button>
          </header>

          <div class="assistant-voice shrink-0 px-4 pb-3 pt-3 text-center" :class="'is-' + voiceMode">
            <div class="assistant-wave mx-auto flex h-[74px] max-w-[270px] items-center justify-center gap-[3px] overflow-hidden" aria-hidden="true">
              <span
                v-for="index in bars" :key="index"
                class="assistant-wave-bar min-w-[2px] max-w-1 flex-1 rounded-full"
                :style="{ height: barHeight(index), animationDelay: '-' + index * 87 + 'ms', animationDuration: 950 + (index % 5) * 140 + 'ms' }"
              />
            </div>
            <p class="mt-1 text-xs font-semibold" role="status">{{ assistant.state === 'ready' ? t('assistant.readyVoice') : t(`assistant.state.${assistant.state}`) }}</p>
            <p class="mt-1 text-[11px] text-muted-foreground">
              {{ assistant.state === 'ready' ? t('assistant.voiceHint') : assistant.state === 'listening' ? t('assistant.listeningHint') : assistant.state === 'speaking' ? t('assistant.speakingHint') : assistant.state === 'processing' ? t('assistant.processingHint') : '' }}
            </p>
          </div>

          <div class="mx-3 mb-2 flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border bg-muted/35">
            <div class="flex h-9 shrink-0 items-center justify-between border-b px-3">
              <span class="text-xs font-semibold text-muted-foreground">{{ historyOpen ? t('assistant.history') : t('assistant.liveConversation') }}</span>
              <button type="button" class="inline-flex items-center gap-1 rounded-md px-1.5 py-1 text-[11px] font-medium text-primary hover:bg-primary/10 focus-visible:outline-2 focus-visible:outline-ring"
                :aria-pressed="historyOpen" @click="historyOpen = !historyOpen">
                <History class="size-3.5" aria-hidden="true" />
                {{ historyOpen ? t('assistant.latest') : t('assistant.history') }}
              </button>
            </div>
            <div ref="transcript" class="min-h-0 flex-1 space-y-3 overflow-y-auto px-3 py-3" role="log" aria-live="polite" :aria-label="t('assistant.conversation')">
              <p v-if="!displayedMessages.length" class="py-6 text-center text-xs text-muted-foreground">{{ t('assistant.emptyVoice') }}</p>
              <article v-for="message in displayedMessages" :key="message.id" class="flex flex-col gap-1" :class="message.role === 'user' ? 'items-end' : 'items-start'">
                <span class="px-1 text-[10px] text-muted-foreground">{{ message.role === 'user' ? t('assistant.you') : t('assistant.agentLabel') }}</span>
                <div class="assistant-message max-w-[95%] rounded-xl px-3 py-2 text-[13px] leading-relaxed" :class="message.role === 'user' ? 'rounded-br-sm bg-primary/12' : 'rounded-bl-sm border bg-card'">
                  <p class="whitespace-pre-wrap break-words" :class="{ 'line-clamp-4': !historyOpen && message.role === 'assistant', 'line-clamp-2': !historyOpen && message.role === 'user' }">{{ message.text }}</p>
                  <span v-if="message.role === 'assistant' && assistant.state === 'speaking' && !assistant.muted && message.id === latestMessages.at(-1)?.id" class="mt-2 inline-flex items-center gap-1 text-[10px] text-primary">
                    <Volume2 class="size-3" aria-hidden="true" />{{ t('assistant.playing') }}
                  </span>
                </div>
              </article>
              <div v-if="assistant.state === 'processing' && (!latestMessages.length || latestMessages.at(-1)?.role !== 'assistant')" class="flex items-center gap-2 text-xs text-muted-foreground">
                <LoaderCircle class="size-3.5 animate-spin motion-reduce:animate-none" aria-hidden="true" />{{ t('assistant.processingHint') }}
              </div>
            </div>
          </div>

          <footer class="shrink-0 space-y-2 border-t px-3 py-3">
            <div v-if="assistant.error" role="alert" class="flex items-start gap-2 rounded-lg bg-danger/10 px-2.5 py-2 text-xs text-danger">
              <span class="min-w-0 flex-1">{{ t('assistant.friendlyError') }}</span>
              <button type="button" class="shrink-0 underline" :aria-label="t('common.close')" @click="assistant.error = ''">{{ t('common.close') }}</button>
            </div>
            <div v-if="canReconnect" class="flex items-center justify-between gap-2 text-xs">
              <RouterLink to="/system" class="text-primary underline" @click="collapse()">{{ t('assistant.settings') }}</RouterLink>
              <Button size="sm" variant="outline" @click="assistant.connect()">{{ t('common.retry') }}</Button>
            </div>
            <form v-if="textMode" class="flex items-end gap-2" @submit.prevent="send">
              <Button size="icon" type="button" variant="outline" class="size-10 shrink-0" :aria-label="t('assistant.voiceMode')" @click="toggleTextMode"><Mic class="size-4" aria-hidden="true" /></Button>
              <textarea ref="input" v-model="draft" rows="1" class="admin-input max-h-24 min-h-10 min-w-0 flex-1 resize-y py-2 text-sm" :aria-label="t('assistant.input')" :placeholder="t('assistant.input')" :disabled="assistant.state !== 'ready'" @keydown.enter.exact.prevent="send" />
              <Button size="icon" type="submit" class="size-10 shrink-0" :disabled="assistant.state !== 'ready' || !draft.trim()" :aria-label="t('assistant.send')"><Send class="size-4" aria-hidden="true" /></Button>
            </form>
            <div v-else class="relative flex h-[60px] items-center justify-between px-2">
              <Button size="icon" variant="outline" class="size-10 rounded-full" :aria-label="t('assistant.textMode')" @click="toggleTextMode">
                <Keyboard class="size-4" aria-hidden="true" />
              </Button>
              <button
                type="button"
                class="assistant-microphone absolute left-1/2 grid size-[60px] -translate-x-1/2 place-items-center rounded-full bg-primary text-primary-foreground shadow-md focus-visible:outline-2 focus-visible:outline-offset-4 disabled:cursor-not-allowed disabled:opacity-40"
                :class="{ 'is-listening': assistant.micEnabled }"
                :disabled="!assistant.micEnabled && (!micSupported || assistant.state !== 'ready')"
                :aria-label="assistant.micEnabled ? t('assistant.stopMic') : t('assistant.mic')"
                :aria-pressed="assistant.micEnabled"
                :title="!micSupported ? t('assistant.micUnsupported') : undefined"
                @click="microphone"
              >
                <Square v-if="assistant.micEnabled" class="size-5" fill="currentColor" aria-hidden="true" />
                <Mic v-else class="size-6" aria-hidden="true" />
              </button>
              <Button size="icon" variant="outline" class="ml-auto size-10 rounded-full" :aria-label="assistant.muted ? t('assistant.unmute') : t('assistant.mute')" :aria-pressed="!assistant.muted" @click="assistant.toggleSpeaker()">
                <VolumeX v-if="assistant.muted" class="size-4" aria-hidden="true" />
                <Volume2 v-else class="size-4" aria-hidden="true" />
              </Button>
            </div>
            <button v-if="active && assistant.state !== 'listening'" type="button" class="mx-auto flex items-center gap-1.5 rounded-md px-2 py-1 text-[11px] text-muted-foreground hover:bg-muted disabled:opacity-50" :disabled="assistant.state === 'aborting'" @click="assistant.interrupt()">
              <Square class="size-3" aria-hidden="true" />{{ t('assistant.interrupt') }}
            </button>
          </footer>
        </section>
      </Transition>
    </div>
  </Teleport>
</template>

<style scoped>
.assistant-launcher { right: 20px; bottom: calc(20px + env(safe-area-inset-bottom)); }
.assistant-panel-enter-active { transition: opacity 280ms ease-out, transform 280ms cubic-bezier(.22, 1, .36, 1); }
.assistant-panel-leave-active { transition: opacity 220ms ease-in, transform 220ms ease-in; pointer-events: none; }
.assistant-panel-enter-from, .assistant-panel-leave-to { opacity: 0; transform: scale(0); }
.assistant-launcher-active { box-shadow: 0 0 0 5px color-mix(in oklch, var(--primary) 18%, transparent), 0 10px 30px rgb(0 0 0 / 16%); }
.assistant-launcher-listening { box-shadow: 0 0 0 6px color-mix(in oklch, var(--primary) 25%, transparent), 0 10px 30px rgb(0 0 0 / 18%); }
.assistant-launcher-dot { position: absolute; bottom: 1px; right: 1px; width: 12px; height: 12px; border: 2px solid var(--card); border-radius: 50%; }
.assistant-connection-dot { width: 7px; height: 7px; border-radius: 50%; }
.is-online { background: #34c99b; }
.is-connecting { background: #e5b35e; }
.is-offline { background: #ec6f78; }
.assistant-panel { right: 20px; bottom: calc(88px + env(safe-area-inset-bottom)); width: min(375px, calc(100vw - 24px)); height: min(490px, calc(100dvh - 108px)); border-radius: 20px; transform-origin: calc(100% - 28px) calc(100% + 40px); }
.assistant-voice { background: radial-gradient(ellipse at 50% 35%, color-mix(in oklch, var(--primary) 12%, transparent), transparent 74%); }
.assistant-wave-bar { background: linear-gradient(180deg, var(--studio-violet, var(--primary)), var(--studio-cyan, var(--primary))); transition: height 100ms linear; }
.assistant-voice.is-speaking .assistant-wave-bar { animation: assistant-speaking 1s ease-in-out infinite alternate; transform-origin: center; }
.assistant-voice.is-thinking .assistant-wave-bar { animation: assistant-thinking 1.6s ease-in-out infinite alternate; transform-origin: center; opacity: .65; }
.assistant-voice.is-idle .assistant-wave-bar { opacity: .5; }
.assistant-microphone.is-listening { background: var(--danger); color: white; box-shadow: 0 0 0 5px color-mix(in oklch, var(--danger) 18%, transparent); }
@keyframes assistant-speaking { from { transform: scaleY(.36); opacity: .55; } to { transform: scaleY(1); opacity: 1; } }
@keyframes assistant-thinking { from { transform: scaleY(.65); } to { transform: scaleY(1.2); } }
@media (max-width: 639px) {
  .assistant-launcher { right: 12px; bottom: calc(12px + env(safe-area-inset-bottom)); }
  .assistant-panel { right: 12px; bottom: calc(78px + env(safe-area-inset-bottom)); height: min(490px, calc(100dvh - 98px)); transform-origin: calc(100% - 28px) calc(100% + 38px); }
}
@media (prefers-reduced-motion: reduce) {
  .assistant-panel-enter-active, .assistant-panel-leave-active { transition: none; }
  .assistant-wave-bar { animation: none !important; transition: none !important; }
}
</style>

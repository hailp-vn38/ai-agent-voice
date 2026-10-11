<script setup lang="ts">
import { Bot, Mic, MicOff, Minus, Send, Square, Volume2, VolumeX, X } from '@lucide/vue'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { AssistantAudio } from '@/lib/assistant/audio'
import { useAssistantStore } from '@/stores/assistant'
import { useAuthStore } from '@/stores/auth'

const assistant = useAssistantStore()
const auth = useAuthStore()
const route = useRoute()
const { t } = useI18n()
const draft = ref('')
const launcher = ref<HTMLElement>()
const panel = ref<HTMLElement>()
const input = ref<HTMLTextAreaElement>()
const transcript = ref<HTMLElement>()
const mobile = ref(false)
const contextVisible = ref(true)
const context = computed(() => {
  const resource = Object.values(route.params).find(value => typeof value === 'string')
  const page = route.path.split('/')[1] || 'overview'
  return resource ? `${page}: ${resource}` : page
})
const active = computed(() => ['listening', 'processing', 'speaking', 'aborting'].includes(assistant.state))
const micSupported = AssistantAudio.microphoneSupported()
watch(() => route.path, () => { contextVisible.value = true })
watch(() => auth.adminToken, () => { assistant.disconnect(); assistant.visible = false })
watch(() => assistant.visible, async open => {
  if (mobile.value) document.body.style.overflow = open ? 'hidden' : ''
  await nextTick()
  if (open) { if (!input.value?.disabled) input.value?.focus(); else panel.value?.focus() } else launcher.value?.focus()
})
watch(() => assistant.messages.map(m => m.text).join(''), async () => {
  await nextTick()
  transcript.value?.scrollTo({ top: transcript.value.scrollHeight })
})
function collapse() { assistant.visible = false }
function disconnect() { assistant.disconnect(); draft.value = ''; collapse() }
async function send() {
  const text = draft.value.trim()
  if (!text || assistant.state !== 'ready') return
  draft.value = ''
  await assistant.sendText(text)
}
function keydown(event: KeyboardEvent) {
  if (event.defaultPrevented || (document.activeElement instanceof HTMLElement && !panel.value?.contains(document.activeElement) && document.activeElement !== launcher.value)) return
  if (event.key === 'Escape' && assistant.visible) { event.preventDefault(); collapse() }
  if (event.key !== 'Tab' || !mobile.value || !assistant.visible) return
  const elements = Array.from(panel.value?.querySelectorAll<HTMLElement>('button:not(:disabled), a, textarea:not(:disabled)') ?? [])
  const first = elements[0], last = elements.at(-1)
  if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus() }
  else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus() }
}
function visibility() { if (document.hidden && assistant.state === 'listening') assistant.interrupt() }
function resize() { mobile.value = window.matchMedia('(max-width: 639px)').matches }
onMounted(() => { resize(); window.addEventListener('resize', resize); document.addEventListener('visibilitychange', visibility); window.addEventListener('keydown', keydown) })
onBeforeUnmount(() => {
  assistant.disconnect()
  document.body.style.overflow = ''
  window.removeEventListener('resize', resize)
  document.removeEventListener('visibilitychange', visibility)
  window.removeEventListener('keydown', keydown)
})
</script>

<template>
  <Teleport to="body">
    <div v-if="auth.adminToken" class="text-foreground">
      <button ref="launcher" class="assistant-launcher fixed z-30 flex size-14 items-center justify-center rounded-full bg-primary text-primary-foreground shadow-lg focus-visible:outline-2 focus-visible:outline-offset-4" :aria-label="t('assistant.open')" :aria-expanded="assistant.visible" aria-controls="studio-assistant" @click="assistant.visible ? collapse() : assistant.open()"><Bot class="size-6" /></button>
      <Transition enter-active-class="transition duration-150 motion-reduce:transition-none" enter-from-class="opacity-0 scale-95" leave-active-class="transition duration-150 motion-reduce:transition-none" leave-to-class="opacity-0 scale-95">
        <section v-if="assistant.visible" id="studio-assistant" ref="panel" tabindex="-1" class="assistant-panel fixed z-40 flex flex-col overflow-hidden border bg-card shadow-xl" role="dialog" :aria-modal="mobile || undefined" :aria-label="t('assistant.title')">
          <header class="flex shrink-0 items-center gap-2 border-b p-3">
            <Bot class="size-5" /><strong class="flex-1">{{ t('assistant.title') }}</strong>
            <Button size="icon" variant="ghost" :aria-label="t('assistant.collapse')" @click="collapse"><Minus class="size-4" /></Button>
            <Button size="icon" variant="ghost" :aria-label="t('assistant.disconnect')" @click="disconnect"><X class="size-4" /></Button>
          </header>
          <div class="shrink-0 space-y-1 border-b px-3 py-2 text-xs">
            <p role="status">{{ t(`assistant.state.${assistant.state}`) }}</p>
            <p v-if="assistant.settings?.agent_key">{{ assistant.settings.agent_key }} · {{ assistant.settings.template_key ?? t('assistant.agentDefault') }}</p>
            <p v-if="contextVisible" class="flex items-center gap-2">{{ t('assistant.context') }}: {{ context }} <button :aria-label="t('common.close')" @click="contextVisible = false">×</button></p>
          </div>
          <div ref="transcript" class="min-h-0 flex-1 space-y-3 overflow-y-auto p-3" role="log" aria-live="polite" :aria-label="t('assistant.conversation')">
            <p v-if="!assistant.messages.length" class="text-sm text-muted-foreground">{{ t('assistant.empty') }}</p>
            <article v-for="message in assistant.messages" :key="message.id" class="rounded-lg p-3 text-sm" :class="message.role === 'user' ? 'ml-8 bg-primary/10' : 'mr-8 bg-muted'">
              <p class="whitespace-pre-wrap break-words">{{ message.text }}</p>
              <small v-if="['pending', 'failed', 'cancelled', 'uncertain'].includes(message.status)" class="text-muted-foreground">{{ t(`assistant.message.${message.status}`) }}</small>
            </article>
          </div>
          <footer class="shrink-0 space-y-2 border-t p-3">
            <p v-if="assistant.error" role="alert" class="text-xs text-danger">{{ t('assistant.error') }}: {{ assistant.error }}</p>
            <p class="text-xs text-muted-foreground">{{ t('assistant.retention') }}</p>
            <div v-if="['unconfigured', 'disabled', 'error', 'uncertain', 'disconnected', 'auth_error'].includes(assistant.state)" class="flex gap-2">
              <RouterLink to="/system" class="text-sm underline" @click="collapse">{{ t('assistant.settings') }}</RouterLink>
              <Button size="sm" variant="outline" @click="assistant.connect()">{{ t('common.retry') }}</Button>
            </div>
            <div v-if="assistant.state === 'ready'" class="flex flex-wrap gap-1">
              <Button size="sm" variant="outline" @click="draft = t('assistant.suggestSystem')">{{ t('assistant.suggestSystem') }}</Button>
              <Button size="sm" variant="outline" @click="draft = `${t('assistant.suggestContext')}: ${context}`">{{ t('assistant.suggestContext') }}</Button>
            </div>
            <form class="flex items-end gap-2" @submit.prevent="send">
              <textarea ref="input" v-model="draft" class="admin-input min-h-12 min-w-0 flex-1 resize-none" rows="2" :aria-label="t('assistant.input')" :placeholder="t('assistant.input')" :disabled="assistant.state !== 'ready'" @keydown.enter.exact.prevent="send" />
              <Button size="icon" type="submit" :disabled="assistant.state !== 'ready' || !draft.trim()" :aria-label="t('assistant.send')"><Send class="size-4" /></Button>
            </form>
            <meter v-if="assistant.state === 'listening'" :value="assistant.level" min="0" max="1" :aria-label="t('assistant.mic')" class="w-full" />
            <div class="flex gap-2">
              <Button v-if="assistant.state === 'listening'" size="sm" variant="outline" @click="assistant.stopMicrophone()"><MicOff class="size-4" />{{ t('assistant.stopMic') }}</Button>
              <Button v-else size="sm" variant="outline" :disabled="assistant.state !== 'ready' || !micSupported" :title="!micSupported ? t('assistant.micUnsupported') : undefined" @click="assistant.startMicrophone()"><Mic class="size-4" />{{ t('assistant.mic') }}</Button>
              <Button size="icon" variant="outline" :aria-label="assistant.muted ? t('assistant.unmute') : t('assistant.mute')" :aria-pressed="!assistant.muted" @click="assistant.toggleSpeaker()"><VolumeX v-if="assistant.muted" class="size-4" /><Volume2 v-else class="size-4" /></Button>
              <Button size="sm" variant="outline" :disabled="!active || assistant.state === 'aborting'" @click="assistant.interrupt()"><Square class="size-4" />{{ t('assistant.interrupt') }}</Button>
            </div>
          </footer>
        </section>
      </Transition>
    </div>
  </Teleport>
</template>

<style scoped>
.assistant-launcher { right: 24px; bottom: calc(24px + env(safe-area-inset-bottom)); }
.assistant-panel { right: 24px; bottom: calc(92px + env(safe-area-inset-bottom)); width: min(420px, calc(100vw - 32px)); height: min(650px, calc(100dvh - 110px)); border-radius: 16px; }
@media (max-width: 639px) {
  .assistant-panel { inset: 0; width: 100%; height: 100dvh; border-radius: 0; padding-bottom: env(safe-area-inset-bottom); }
}
</style>

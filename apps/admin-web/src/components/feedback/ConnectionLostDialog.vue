<script setup lang="ts">
import { RefreshCw, Unplug } from '@lucide/vue'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, useId, watch } from 'vue'

import { apiUrl } from '@/api/client'
import { useI18n } from '@/composables/useI18n'
import { dismissConnectionLost, reportConnectionRestored, useNotifyState } from '@/composables/useNotify'

/** Seconds between automatic checks; the last value repeats. */
const BACKOFF = [5, 10, 20, 30]

const { t } = useI18n()
const state = useNotifyState()
const titleId = useId()
const bodyId = useId()

const attempts = ref(0)
const checking = ref(false)
const secondsLeft = ref(BACKOFF[0]!)
const retryButton = ref<HTMLButtonElement>()
let ticker: number | undefined
let restoreFocusTo: HTMLElement | null = null

const open = computed(() => state.connectionLost)

function schedule() {
  secondsLeft.value = BACKOFF[Math.min(attempts.value, BACKOFF.length - 1)]!
}

async function check() {
  if (checking.value) return
  checking.value = true
  attempts.value += 1
  try {
    // Bypass the API client: this probe must not feed back into the outage state.
    const response = await fetch(apiUrl('/health'), { headers: { Accept: 'text/plain' }, cache: 'no-store' })
    if (response.ok && (await response.text()).trim() === 'ok') {
      reportConnectionRestored()
      return
    }
  } catch {
    // Still unreachable.
  } finally {
    checking.value = false
  }
  schedule()
}

function tick() {
  if (checking.value || document.visibilityState === 'hidden') return
  secondsLeft.value -= 1
  if (secondsLeft.value <= 0) void check()
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && open.value) dismissConnectionLost()
}

function onBrowserOnline() {
  if (open.value) void check()
}

watch(open, async (value) => {
  window.clearInterval(ticker)
  if (value) {
    attempts.value = 0
    schedule()
    ticker = window.setInterval(tick, 1000)
    restoreFocusTo = document.activeElement as HTMLElement | null
    await nextTick()
    retryButton.value?.focus()
  } else {
    restoreFocusTo?.focus?.()
    restoreFocusTo = null
  }
})

onMounted(() => {
  window.addEventListener('keydown', onKeydown)
  window.addEventListener('online', onBrowserOnline)
})
onBeforeUnmount(() => {
  window.clearInterval(ticker)
  window.removeEventListener('keydown', onKeydown)
  window.removeEventListener('online', onBrowserOnline)
})
</script>

<template>
  <Teleport to="body">
    <Transition
      enter-active-class="transition duration-200 ease-out"
      enter-from-class="opacity-0"
      leave-active-class="transition duration-150 ease-in"
      leave-to-class="opacity-0"
    >
      <div v-if="open" class="fixed inset-0 z-[60] flex items-center justify-center p-4">
        <div class="absolute inset-0 bg-black/55 backdrop-blur-sm" aria-hidden="true" />
        <section
          role="alertdialog"
          aria-modal="true"
          :aria-labelledby="titleId"
          :aria-describedby="bodyId"
          class="relative w-full max-w-md overflow-hidden rounded-3xl border border-border/80 bg-card text-card-foreground shadow-2xl"
        >
          <div class="connection-glow absolute inset-x-0 top-0 h-40" aria-hidden="true" />

          <div class="relative flex flex-col items-center px-6 pt-8 pb-6 text-center sm:px-8">
            <div class="relative flex size-20 items-center justify-center" aria-hidden="true">
              <span class="connection-ring absolute inset-0 rounded-full border-2 border-danger/40" />
              <span class="connection-ring connection-ring-late absolute inset-0 rounded-full border-2 border-danger/30" />
              <span class="relative flex size-16 items-center justify-center rounded-full bg-danger/12 text-danger">
                <Unplug class="size-8" />
              </span>
            </div>

            <h2 :id="titleId" class="mt-5 text-xl font-semibold tracking-tight">{{ t('feedback.connection.title') }}</h2>
            <p :id="bodyId" class="mt-2 text-sm leading-relaxed text-muted-foreground">{{ t('feedback.connection.body') }}</p>

            <div class="mt-5 inline-flex items-center gap-2 rounded-full bg-surface px-3.5 py-1.5 text-xs font-medium text-muted-foreground" role="status" aria-live="polite">
              <RefreshCw class="size-3.5" :class="checking ? 'animate-spin text-studio-cyan' : ''" aria-hidden="true" />
              <span>{{ checking ? t('feedback.connection.checking') : t('feedback.connection.next', { seconds: secondsLeft }) }}</span>
              <span v-if="attempts > 0" class="text-muted-foreground/70">· {{ t('feedback.connection.attempts', { count: attempts }) }}</span>
            </div>
          </div>

          <footer class="flex flex-col-reverse gap-2 border-t border-border/70 bg-surface/50 px-6 py-4 sm:flex-row sm:justify-end sm:px-8">
            <button
              type="button"
              class="inline-flex h-10 items-center justify-center rounded-xl px-4 text-sm font-medium text-muted-foreground transition hover:bg-accent hover:text-foreground"
              @click="dismissConnectionLost()"
            >
              {{ t('feedback.connection.dismiss') }}
            </button>
            <button
              ref="retryButton"
              type="button"
              class="inline-flex h-10 items-center justify-center gap-2 rounded-xl bg-primary px-5 text-sm font-semibold text-primary-foreground shadow-sm transition hover:opacity-90 disabled:opacity-60"
              :disabled="checking"
              @click="check()"
            >
              <RefreshCw class="size-4" :class="checking ? 'animate-spin' : ''" aria-hidden="true" />
              {{ t('feedback.connection.retry') }}
            </button>
          </footer>
        </section>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.connection-glow {
  background: radial-gradient(70% 100% at 50% 0%, color-mix(in oklab, var(--danger) 18%, transparent), transparent 70%);
}

.connection-ring {
  animation: connection-ring 2.4s ease-out infinite;
}

.connection-ring-late {
  animation-delay: 1.2s;
}

@keyframes connection-ring {
  0% { transform: scale(0.8); opacity: 0.9; }
  100% { transform: scale(1.5); opacity: 0; }
}

@media (prefers-reduced-motion: reduce) {
  .connection-ring {
    animation: none;
    opacity: 0.4;
  }
}
</style>

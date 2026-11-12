<script setup lang="ts">
import { AlertTriangle, Check, CheckCircle2, Copy, Info, X, XCircle } from '@lucide/vue'
import { ref, type Component } from 'vue'

import { useI18n } from '@/composables/useI18n'
import { dismiss, useNotifyState, type Toast, type ToastTone } from '@/composables/useNotify'
import type { MessageKey } from '@/i18n/messages'

const { t } = useI18n()
const state = useNotifyState()

// Hovering or focusing the stack pauses every countdown so a toast never vanishes mid-read.
const paused = ref(false)
const copiedId = ref<number | null>(null)

const tones: Record<ToastTone, { icon: Component; title: MessageKey; accent: string; badge: string; bar: string }> = {
  error: { icon: XCircle, title: 'feedback.error', accent: 'before:bg-danger', badge: 'bg-danger/12 text-danger', bar: 'bg-danger' },
  warning: { icon: AlertTriangle, title: 'feedback.warning', accent: 'before:bg-warning', badge: 'bg-warning/15 text-warning', bar: 'bg-warning' },
  success: { icon: CheckCircle2, title: 'feedback.success', accent: 'before:bg-success', badge: 'bg-success/12 text-success', bar: 'bg-success' },
  info: { icon: Info, title: 'feedback.info', accent: 'before:bg-studio-cyan', badge: 'bg-studio-cyan/12 text-studio-cyan', bar: 'bg-studio-cyan' },
}

async function copyRequestId(toast: Toast) {
  if (!toast.requestId) return
  try {
    await navigator.clipboard.writeText(toast.requestId)
    copiedId.value = toast.id
    window.setTimeout(() => {
      if (copiedId.value === toast.id) copiedId.value = null
    }, 1500)
  } catch {
    // Clipboard can be blocked; the id stays visible to copy by hand.
  }
}

function runAction(toast: Toast) {
  const action = toast.action
  // The action owns the outcome now; a new failure will raise a fresh toast.
  toast.onDismiss = undefined
  dismiss(toast.id)
  void Promise.resolve().then(() => action?.run())
}
</script>

<template>
  <Teleport to="body">
    <section
      :aria-label="t('feedback.region')"
      class="pointer-events-none fixed inset-x-0 top-0 z-[70] flex flex-col items-center gap-2 p-3 sm:inset-x-auto sm:right-0 sm:items-end sm:p-5"
      @mouseenter="paused = true"
      @mouseleave="paused = false"
      @focusin="paused = true"
      @focusout="paused = false"
    >
      <TransitionGroup
        enter-active-class="transition duration-300 ease-out"
        enter-from-class="opacity-0 -translate-y-2 sm:translate-y-0 sm:translate-x-6"
        leave-active-class="transition duration-200 ease-in absolute"
        leave-to-class="opacity-0 scale-95"
        move-class="transition duration-300"
      >
        <article
          v-for="toast in state.toasts"
          :key="toast.id"
          :role="toast.tone === 'error' || toast.tone === 'warning' ? 'alert' : 'status'"
          :data-toast-tone="toast.tone"
          class="pointer-events-auto relative w-full max-w-sm overflow-hidden rounded-2xl border border-border/80 bg-card/95 text-card-foreground shadow-xl shadow-black/10 backdrop-blur-md before:absolute before:inset-y-0 before:left-0 before:w-1 sm:w-96"
          :class="tones[toast.tone].accent"
        >
          <div class="flex gap-3 p-4 pl-5">
            <span class="flex size-9 shrink-0 items-center justify-center rounded-xl" :class="tones[toast.tone].badge">
              <component :is="tones[toast.tone].icon" class="size-5" aria-hidden="true" />
            </span>

            <div class="min-w-0 flex-1">
              <div class="flex items-center gap-2">
                <p class="truncate text-sm font-semibold">{{ toast.title ?? t(tones[toast.tone].title) }}</p>
                <span
                  v-if="toast.count > 1"
                  class="rounded-full bg-muted px-1.5 text-[11px] font-semibold tabular-nums text-muted-foreground"
                >×{{ toast.count }}</span>
              </div>
              <p class="mt-0.5 break-words text-sm leading-snug text-muted-foreground">{{ toast.message }}</p>

              <button
                v-if="toast.requestId"
                type="button"
                class="mt-2 inline-flex max-w-full items-center gap-1.5 rounded-md bg-surface px-2 py-1 font-mono text-[11px] text-muted-foreground transition hover:text-foreground"
                :title="t('feedback.copy')"
                @click="copyRequestId(toast)"
              >
                <span class="shrink-0 font-sans">{{ t('feedback.requestId') }}</span>
                <span class="truncate">{{ toast.requestId }}</span>
                <Check v-if="copiedId === toast.id" class="size-3 shrink-0 text-success" aria-hidden="true" />
                <Copy v-else class="size-3 shrink-0" aria-hidden="true" />
                <span v-if="copiedId === toast.id" class="sr-only">{{ t('feedback.copied') }}</span>
              </button>

              <div v-if="toast.action" class="mt-3">
                <button
                  type="button"
                  class="inline-flex h-8 items-center rounded-lg bg-foreground px-3 text-xs font-semibold text-background transition hover:opacity-90"
                  @click="runAction(toast)"
                >
                  {{ toast.action.label }}
                </button>
              </div>
            </div>

            <button
              type="button"
              class="-mt-1 -mr-1 flex size-7 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition hover:bg-accent hover:text-foreground"
              :aria-label="t('feedback.dismiss')"
              @click="dismiss(toast.id)"
            >
              <X class="size-4" aria-hidden="true" />
            </button>
          </div>

          <div v-if="toast.duration > 0" class="absolute inset-x-0 bottom-0 h-0.5 bg-transparent">
            <div
              :key="toast.version"
              class="toast-countdown h-full opacity-60"
              :class="tones[toast.tone].bar"
              :style="{ animationDuration: `${toast.duration}ms`, animationPlayState: paused ? 'paused' : 'running' }"
              @animationend="dismiss(toast.id)"
            />
          </div>
        </article>
      </TransitionGroup>
    </section>
  </Teleport>
</template>

<style scoped>
.toast-countdown {
  transform-origin: left;
  animation-name: toast-countdown;
  animation-timing-function: linear;
  animation-fill-mode: forwards;
}

@keyframes toast-countdown {
  from { transform: scaleX(1); }
  to { transform: scaleX(0); }
}
</style>

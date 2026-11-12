import { onScopeDispose, reactive, readonly, watch } from 'vue'

import { translate } from '@/composables/useI18n'

/**
 * App-wide feedback: toasts for errors and outcomes, plus one connection-lost
 * dialog when the server stops answering. Module state, so stores, API code
 * and components all report into the same place.
 */

export type ToastTone = 'error' | 'warning' | 'success' | 'info'

export interface ToastAction {
  label: string
  run: () => unknown
}

export interface ToastOptions {
  title?: string
  action?: ToastAction
  /** Milliseconds before auto-dismiss; 0 keeps the toast until closed. */
  duration?: number
  /** Toasts with the same key replace each other instead of stacking. */
  key?: string
  onDismiss?: () => void
}

export interface Toast {
  id: number
  key: string
  tone: ToastTone
  title?: string
  message: string
  /** Server request id pulled out of the message, shown separately for support. */
  requestId?: string
  action?: ToastAction
  duration: number
  /** How many times the same toast fired while visible. */
  count: number
  /** Bumped on every repeat so the viewport restarts its countdown. */
  version: number
  onDismiss?: () => void
}

const DEFAULT_DURATION: Record<ToastTone, number> = {
  error: 8_000,
  warning: 7_000,
  success: 4_000,
  info: 5_000,
}
const MAX_TOASTS = 4
const REQUEST_ID = /\s*\(request_id: ([^)]+)\)\s*$/

const state = reactive({
  toasts: [] as Toast[],
  connectionLost: false,
})

let nextId = 1
const reconnectHandlers = new Set<() => unknown>()

function push(tone: ToastTone, rawMessage: string, options: ToastOptions = {}): number | null {
  const trimmed = rawMessage.trim()
  if (!trimmed) return null
  // The dialog already explains an outage; per-request failures would only add noise.
  if (state.connectionLost && tone === 'error') return null

  const match = trimmed.match(REQUEST_ID)
  const message = match ? trimmed.slice(0, match.index).trim() : trimmed
  const key = options.key ?? `${tone}:${message}`
  const existing = state.toasts.find((toast) => toast.key === key)
  if (existing) {
    existing.count = existing.message === message ? existing.count + 1 : 1
    Object.assign(existing, { message, requestId: match?.[1], title: options.title ?? existing.title, action: options.action ?? existing.action })
    existing.version += 1
    return existing.id
  }

  const toast: Toast = {
    id: nextId++,
    key,
    tone,
    title: options.title,
    message,
    requestId: match?.[1],
    action: options.action,
    duration: options.duration ?? DEFAULT_DURATION[tone],
    count: 1,
    version: 0,
    onDismiss: options.onDismiss,
  }
  state.toasts.push(toast)
  while (state.toasts.length > MAX_TOASTS) dismiss(state.toasts[0]!.id)
  return toast.id
}

export function dismiss(id: number) {
  const index = state.toasts.findIndex((toast) => toast.id === id)
  if (index === -1) return
  const [toast] = state.toasts.splice(index, 1)
  toast?.onDismiss?.()
}

export function dismissByKey(key: string) {
  const toast = state.toasts.find((item) => item.key === key)
  if (toast) dismiss(toast.id)
}

export const notify = {
  error: (message: string, options?: ToastOptions) => push('error', message, options),
  warning: (message: string, options?: ToastOptions) => push('warning', message, options),
  success: (message: string, options?: ToastOptions) => push('success', message, options),
  info: (message: string, options?: ToastOptions) => push('info', message, options),
  dismiss,
  clear: () => [...state.toasts].forEach((toast) => dismiss(toast.id)),
}

/** Called by the API client when the server stops answering at the transport level. */
export function reportConnectionLost() {
  if (state.connectionLost) return
  state.connectionLost = true
  // Errors already on screen describe the same outage.
  state.toasts.filter((toast) => toast.tone === 'error').forEach((toast) => dismiss(toast.id))
}

/** Close the dialog after the server answered again, and replay what failed. */
export function reportConnectionRestored() {
  if (!state.connectionLost) return
  state.connectionLost = false
  notify.success(translate('feedback.reconnected'), { key: 'connection' })
  for (const handler of reconnectHandlers) void Promise.resolve().then(handler).catch(() => undefined)
}

/** Close the dialog without a confirmed reconnect; the next failure reopens it. */
export function dismissConnectionLost() {
  state.connectionLost = false
}

export function onReconnect(handler: () => unknown) {
  reconnectHandlers.add(handler)
  return () => reconnectHandlers.delete(handler)
}

export function useNotifyState() {
  return readonly(state)
}

export interface SourceToastOptions {
  title?: string | (() => string)
  /** Adds a "Try again" button and replays automatically after reconnecting. */
  retry?: () => unknown
  /** Runs when the operator closes the toast or it times out, e.g. to clear the source. */
  onDismiss?: () => void
}

/** Alias for error call sites. */
export type ErrorToastOptions = SourceToastOptions

/**
 * Mirror a page's message ref into a toast. The page's own state stays the
 * source of truth: the toast appears when it is set and leaves when it clears.
 */
export function useToastSource(source: () => string | null | undefined, tone: ToastTone, options: SourceToastOptions = {}) {
  const key = `source:${nextId++}`
  const detach = () => {
    const toast = state.toasts.find((item) => item.key === key)
    if (toast) {
      // The source already cleared; do not echo back into it.
      toast.onDismiss = undefined
      dismiss(toast.id)
    }
  }
  watch(
    source,
    (message) => {
      if (!message) return detach()
      push(tone, message, {
        key,
        title: typeof options.title === 'function' ? options.title() : options.title,
        action: options.retry ? { label: translate('common.retry'), run: options.retry } : undefined,
        onDismiss: options.onDismiss,
      })
    },
    { immediate: true },
  )
  if (options.retry) {
    const retry = options.retry
    onScopeDispose(onReconnect(() => (source() ? retry() : undefined)))
  }
  onScopeDispose(detach)
}

export function useErrorToast(source: () => string | null | undefined, options: SourceToastOptions = {}) {
  useToastSource(source, 'error', options)
}

/** Success notices clear their source on close so the same message can show again. */
export function useSuccessToast(source: () => string | null | undefined, options: SourceToastOptions = {}) {
  useToastSource(source, 'success', options)
}

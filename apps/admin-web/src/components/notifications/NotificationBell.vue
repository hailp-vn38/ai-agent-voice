<script setup lang="ts">
import { Bell, CheckCheck, Settings2, Sparkles, Trash2, X } from '@lucide/vue'
import { storeToRefs } from 'pinia'
import { computed, getCurrentInstance, onMounted, onUnmounted, ref } from 'vue'

import { resolveNotificationHref } from '@/api/notifications'
import type {
  NotificationItem,
  NotificationSeverity,
  NotificationTopic,
  OutboxDeliveryStatus,
} from '@/api/types/notifications'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { MessageKey } from '@/i18n/messages'
import { useNotificationsStore } from '@/stores/notifications'

const props = withDefaults(
  defineProps<{
    compact?: boolean
    placement?: 'header' | 'sidebar'
    teleport?: boolean
    onNavigate?: (href: string) => void | Promise<void>
  }>(),
  {
    compact: false,
    placement: 'header',
    teleport: import.meta.env.MODE !== 'test',
  },
)

const instance = getCurrentInstance()
const instanceUid = instance?.uid ?? Math.random()
const { t } = useI18n()
const notifications = useNotificationsStore()
const { filter, items, unreadCount, activeToast, demoMode, toastHostUid } =
  storeToRefs(notifications)
const isToastHost = computed(() => toastHostUid.value === instanceUid)

const triggerEl = ref<HTMLButtonElement | null>(null)
const open = ref(false)
const anchorLeftPx = ref<number | null>(null)
const anchorOrigin = ref<string | null>(null)

const popoverStyle = computed(() => {
  const style: Record<string, string> = {}
  if (props.placement === 'sidebar' && anchorLeftPx.value !== null && anchorLeftPx.value > 0) {
    style.left = `${anchorLeftPx.value}px`
    style.bottom = '16px'
  }
  if (anchorOrigin.value) {
    style.transformOrigin = anchorOrigin.value
  }
  return Object.keys(style).length > 0 ? style : undefined
})

const popoverFallbackClass = computed(() => {
  if (props.placement === 'sidebar') {
    return 'is-sidebar bottom-4 left-4 lg:left-[268px]'
  }
  return 'is-header top-16 right-4'
})

function updateAnchorPosition(): void {
  if (!triggerEl.value) {
    anchorLeftPx.value = null
    anchorOrigin.value = null
    return
  }
  const triggerRect = triggerEl.value.getBoundingClientRect()
  const hasTriggerMetrics = triggerRect.width > 0 || triggerRect.height > 0
  const triggerCenterX = triggerRect.left + triggerRect.width / 2
  const triggerCenterY = triggerRect.top + triggerRect.height / 2

  if (props.placement !== 'sidebar') {
    anchorLeftPx.value = null
    if (hasTriggerMetrics && window.innerWidth > 0) {
      const offsetRight = Math.round(window.innerWidth - 16 - triggerCenterX)
      const offsetTop = Math.round(triggerCenterY - 64)
      anchorOrigin.value = `calc(100% - ${offsetRight}px) ${offsetTop}px`
    } else {
      anchorOrigin.value = null
    }
    return
  }

  const sidebarEl = triggerEl.value.closest('aside')
  const rect = (sidebarEl ?? triggerEl.value).getBoundingClientRect()
  if (rect.right > 0) {
    const leftPx = Math.round(rect.right + 12)
    anchorLeftPx.value = leftPx
    if (hasTriggerMetrics && window.innerHeight > 0) {
      const originX = Math.round(triggerCenterX - leftPx)
      const offsetBottom = Math.round(window.innerHeight - 16 - triggerCenterY)
      anchorOrigin.value = `${originX}px calc(100% - ${offsetBottom}px)`
    } else {
      anchorOrigin.value = null
    }
  } else {
    anchorLeftPx.value = null
    anchorOrigin.value = null
  }
}

function dismissToast(): void {
  notifications.dismissToast()
}

async function toggleOpen(): Promise<void> {
  const nextOpen = !open.value
  if (nextOpen) {
    updateAnchorPosition()
  }
  open.value = nextOpen
  if (nextOpen) {
    await notifications.refresh()
  }
}

async function setFilter(next: 'all' | 'unread'): Promise<void> {
  await notifications.setFilter(next)
}

async function openNotification(item: NotificationItem): Promise<void> {
  if (!item.is_read) {
    await notifications.markRead([item.id])
  } else {
    await notifications.refresh()
  }
  open.value = false
  const href = resolveNotificationHref(item)
  if (href) {
    if (props.onNavigate) {
      await props.onNavigate(href)
    } else {
      const globalRouter = (
        instance?.appContext.config.globalProperties as {
          $router?: { push?: (to: string) => unknown }
        }
      )?.$router
      await globalRouter?.push?.(href)
    }
  }
}

async function markAllRead(): Promise<void> {
  await notifications.markAllRead()
}

async function purgeRead(): Promise<void> {
  await notifications.purgeRead()
}

async function removeItem(id: string): Promise<void> {
  await notifications.removeItem(id)
}

async function simulateDemo(): Promise<void> {
  await notifications.simulateDemo()
}

async function openSettingsPage(): Promise<void> {
  open.value = false
  if (props.onNavigate) {
    await props.onNavigate('/system/notifications')
  } else {
    const globalRouter = (
      instance?.appContext.config.globalProperties as {
        $router?: { push?: (to: string) => unknown }
      }
    )?.$router
    await globalRouter?.push?.('/system/notifications')
  }
}

function topicLabel(topic: NotificationTopic): string {
  return t(`notifications.topic.${topic}` as MessageKey)
}

function statusLabel(status: OutboxDeliveryStatus): string {
  return t(`notifications.status.${status}` as MessageKey)
}

function severityDotClass(severity: NotificationSeverity): string {
  if (severity === 'error') return 'bg-danger'
  if (severity === 'warning') return 'bg-warning'
  if (severity === 'success') return 'bg-success'
  return 'bg-studio-violet'
}

function deliveryStatusClass(status: OutboxDeliveryStatus): string {
  if (status === 'sent') return 'text-success-foreground'
  if (status === 'failed') return 'text-danger-foreground'
  return 'text-muted-foreground'
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape' && open.value) {
    open.value = false
  }
}

onMounted(() => {
  window.addEventListener('keydown', onKeydown)
  notifications.registerBell(instanceUid)
})

onUnmounted(() => {
  window.removeEventListener('keydown', onKeydown)
  notifications.unregisterBell(instanceUid)
})
</script>

<template>
  <div class="relative inline-flex items-center">
    <button
      ref="triggerEl"
      type="button"
      data-testid="notification-bell-trigger"
      class="relative inline-flex items-center justify-center gap-2 rounded-lg text-sidebar-foreground/80 transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      :class="props.compact ? 'size-9' : 'h-9 px-3 text-xs font-medium border border-border bg-background'"
      :aria-label="t('notifications.bellLabel', { count: unreadCount })"
      :aria-expanded="open"
      :title="t('notifications.bellLabel', { count: unreadCount })"
      @click="toggleOpen"
    >
      <Bell class="size-4 shrink-0" />
      <span v-if="!props.compact" class="truncate">{{ t('notifications.title') }}</span>
      <span
        v-if="unreadCount > 0"
        data-testid="notification-unread-badge"
        class="inline-flex min-w-[18px] items-center justify-center rounded-full bg-danger px-1.5 py-0.5 text-[10px] font-bold leading-none text-white shadow-xs"
        :class="props.compact ? 'absolute -right-0.5 -top-0.5' : ''"
      >
        {{ unreadCount > 99 ? '99+' : unreadCount }}
      </span>
    </button>

    <Teleport to="body" :disabled="!props.teleport">
      <!-- Click-outside backdrop -->
      <div
        v-if="open"
        class="fixed inset-0 z-50"
        aria-hidden="true"
        @click="open = false"
      />

      <!-- Inbox Popover (teleported to body so sidebar stacking context never traps it below Floating Chat) -->
      <Transition name="notification-panel">
        <section
          v-if="open"
          data-testid="notification-popover"
          class="notification-popover fixed z-[60] flex w-[360px] max-w-[calc(100vw-2rem)] flex-col rounded-2xl border border-border bg-card p-4 text-card-foreground shadow-2xl sm:w-[400px]"
          :class="popoverFallbackClass"
          :style="popoverStyle"
          role="dialog"
          :aria-label="t('notifications.title')"
        >
          <!-- Header -->
          <div class="flex items-center justify-between gap-2 border-b border-border/70 pb-3">
            <div class="flex min-w-0 items-center gap-2">
              <h3 class="truncate text-sm font-semibold text-foreground">
                {{ t('notifications.title') }}
              </h3>
              <Badge v-if="demoMode" variant="secondary" class="shrink-0 text-[10px]">
                {{ t('notifications.demoBadge') }}
              </Badge>
            </div>

            <div class="flex shrink-0 items-center gap-1">
              <Button
                v-if="demoMode"
                type="button"
                variant="outline"
                size="sm"
                data-testid="notification-simulate-demo"
                class="h-7 gap-1 px-2 text-[11px]"
                @click="simulateDemo"
              >
                <Sparkles class="size-3 text-studio-violet" />
                <span>{{ t('notifications.simulateDemo') }}</span>
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                data-testid="notification-open-settings"
                class="size-7"
                :aria-label="t('notifications.settings.title')"
                :title="t('notifications.settings.title')"
                @click="openSettingsPage"
              >
                <Settings2 class="size-3.5" />
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                class="size-7"
                :aria-label="t('common.close')"
                @click="open = false"
              >
                <X class="size-3.5" />
              </Button>
            </div>
          </div>

          <!-- Filter Tabs & Bulk Actions -->
          <div class="mt-3 flex items-center justify-between gap-2">
            <div class="inline-flex rounded-lg border border-border bg-muted/60 p-0.5 text-xs">
              <button
                type="button"
                data-testid="notification-tab-all"
                class="rounded-md px-2.5 py-1 font-medium transition-colors"
                :class="
                  filter === 'all'
                    ? 'bg-card text-foreground shadow-xs'
                    : 'text-muted-foreground hover:text-foreground'
                "
                @click="setFilter('all')"
              >
                {{ t('notifications.filter.all') }}
              </button>
              <button
                type="button"
                data-testid="notification-tab-unread"
                class="rounded-md px-2.5 py-1 font-medium transition-colors"
                :class="
                  filter === 'unread'
                    ? 'bg-card text-foreground shadow-xs'
                    : 'text-muted-foreground hover:text-foreground'
                "
                @click="setFilter('unread')"
              >
                {{ t('notifications.filter.unread') }} ({{ unreadCount }})
              </button>
            </div>

            <div class="flex items-center gap-1">
              <button
                type="button"
                data-testid="notification-mark-all-read"
                class="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground disabled:opacity-40"
                :title="t('notifications.markAllRead')"
                :disabled="unreadCount === 0"
                @click="markAllRead"
              >
                <CheckCheck class="size-3.5" />
                <span class="hidden sm:inline">{{ t('notifications.markAllRead') }}</span>
              </button>
              <button
                type="button"
                data-testid="notification-purge-read"
                class="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-danger/10 hover:text-danger-foreground"
                :title="t('notifications.purgeRead')"
                @click="purgeRead"
              >
                <Trash2 class="size-3.5" />
                <span class="hidden sm:inline">{{ t('notifications.purgeRead') }}</span>
              </button>
            </div>
          </div>

          <!-- Notification List -->
          <div class="mt-3 max-h-[340px] space-y-2 overflow-y-auto pr-0.5">
            <p
              v-if="items.length === 0"
              class="py-8 text-center text-xs text-muted-foreground"
            >
              {{ t('notifications.empty') }}
            </p>

            <div
              v-for="item in items"
              :key="item.id"
              data-testid="notification-item"
              class="group rounded-xl border p-3 transition-colors"
              :class="
                item.is_read
                  ? 'border-border/60 bg-background/60 opacity-75 hover:bg-surface/60'
                  : 'border-studio-violet/30 bg-surface/90 hover:bg-surface-elevated'
              "
            >
              <div class="flex items-start justify-between gap-2">
                <button
                  type="button"
                  data-testid="notification-open-resource"
                  class="min-w-0 flex-1 text-left"
                  @click="openNotification(item)"
                >
                  <div class="flex items-center gap-2">
                    <span
                      class="size-2 shrink-0 rounded-full"
                      :class="item.is_read ? 'bg-muted-foreground/40' : severityDotClass(item.severity)"
                      :title="!item.is_read ? t('notifications.unreadDot') : undefined"
                    />
                    <span class="truncate text-[11px] font-medium text-muted-foreground">
                      {{ topicLabel(item.topic) }}
                    </span>
                  </div>
                  <p class="mt-1 text-xs font-semibold text-foreground">
                    {{ item.title }}
                  </p>
                  <p class="mt-0.5 line-clamp-2 text-xs text-muted-foreground">
                    {{ item.body }}
                  </p>
                </button>

                <button
                  type="button"
                  data-testid="notification-delete-item"
                  class="inline-flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground opacity-60 transition-colors hover:bg-danger/10 hover:text-danger-foreground group-hover:opacity-100"
                  :aria-label="t('notifications.deleteItem')"
                  :title="t('notifications.deleteItem')"
                  @click.stop="removeItem(item.id)"
                >
                  <X class="size-3.5" />
                </button>
              </div>

              <div
                v-if="item.deliveries.length > 0"
                class="mt-2 flex flex-wrap items-center gap-2 border-t border-border/50 pt-1.5 text-[11px]"
              >
                <span
                  v-for="delivery in item.deliveries"
                  :key="delivery.channel"
                  class="inline-flex items-center gap-1 font-medium"
                  :class="deliveryStatusClass(delivery.status)"
                  :title="delivery.last_error_code ?? undefined"
                >
                  <span class="capitalize">{{ delivery.channel }}:</span>
                  <span>{{ statusLabel(delivery.status) }}</span>
                </span>
              </div>
            </div>
          </div>
        </section>
      </Transition>

      <!-- Live Toast Popup (top-right so Floating Chat at bottom-right never covers it) -->
      <Transition name="notification-toast">
        <div
          v-if="isToastHost && activeToast"
          data-testid="notification-toast"
          class="notification-toast fixed top-4 right-4 z-[60] flex w-80 max-w-[calc(100vw-2rem)] items-start justify-between gap-2.5 rounded-2xl border border-border bg-card p-3.5 text-card-foreground shadow-2xl"
          role="status"
          aria-live="polite"
        >
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-1.5">
              <span class="size-2 shrink-0 rounded-full" :class="severityDotClass(activeToast.severity)" />
              <span class="truncate text-[11px] font-medium text-muted-foreground">
                {{ topicLabel(activeToast.topic) }}
              </span>
            </div>
            <p class="mt-1 text-xs font-semibold text-foreground">
              {{ activeToast.title }}
            </p>
            <p class="mt-0.5 line-clamp-2 text-xs text-muted-foreground">
              {{ activeToast.body }}
            </p>
          </div>
          <button
            type="button"
            class="inline-flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground"
            :aria-label="t('notifications.toastDismiss')"
            @click="dismissToast"
          >
            <X class="size-3.5" />
          </button>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>

<style scoped>
.notification-popover.is-sidebar {
  transform-origin: -28px calc(100% - 76px);
}
.notification-popover.is-header {
  transform-origin: calc(100% - 56px) -36px;
}
.notification-toast {
  transform-origin: calc(100% - 28px) -16px;
}
.notification-panel-enter-active,
.notification-toast-enter-active {
  transition: opacity 280ms ease-out, transform 280ms cubic-bezier(.22, 1, .36, 1);
}
.notification-panel-leave-active,
.notification-toast-leave-active {
  transition: opacity 220ms ease-in, transform 220ms ease-in;
  pointer-events: none;
}
.notification-panel-enter-from,
.notification-panel-leave-to,
.notification-toast-enter-from,
.notification-toast-leave-to {
  opacity: 0;
  transform: scale(0);
}
@media (prefers-reduced-motion: reduce) {
  .notification-panel-enter-active,
  .notification-panel-leave-active,
  .notification-toast-enter-active,
  .notification-toast-leave-active {
    transition: none;
  }
}
</style>

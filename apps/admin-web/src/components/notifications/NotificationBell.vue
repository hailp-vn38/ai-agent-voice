<script lang="ts">
const mountedBellUids = new Set<number>()
</script>

<script setup lang="ts">
import { Bell, CheckCheck, Sparkles, Trash2, X } from '@lucide/vue'
import { computed, getCurrentInstance, onMounted, onUnmounted, ref } from 'vue'

import {
  isNotificationsDemoMode,
  notificationsApi,
  resolveNotificationHref,
  subscribeNotificationEvents,
} from '@/api/notifications'
import type {
  NotificationItem,
  NotificationSeverity,
  NotificationTopic,
  OutboxDeliveryStatus,
} from '@/api/types/notifications'
import { useI18n } from '@/composables/useI18n'
import type { MessageKey } from '@/i18n/messages'

const props = withDefaults(
  defineProps<{
    compact?: boolean
    placement?: 'header' | 'sidebar'
    onNavigate?: (href: string) => void | Promise<void>
  }>(),
  {
    compact: false,
    placement: 'header',
  },
)

const instance = getCurrentInstance()
const instanceUid = instance?.uid ?? Math.random()
const isToastHost = ref(false)
const { t } = useI18n()

const open = ref(false)
const filter = ref<'all' | 'unread'>('all')
const items = ref<NotificationItem[]>([])
const unreadCount = ref(0)
const toastEnabled = ref(true)
const activeToast = ref<NotificationItem | null>(null)
const demoMode = ref(false)
let toastTimer: ReturnType<typeof setTimeout> | null = null
let unsubscribeEvents: (() => void) | null = null

const popoverPositionClass = computed(() => {
  if (props.placement === 'sidebar') {
    return 'bottom-full left-0 mb-2 w-[360px] sm:w-[400px]'
  }
  return 'top-full right-0 mt-2 w-[340px] sm:w-[400px]'
})

async function refresh(): Promise<void> {
  const [page, settings] = await Promise.all([
    notificationsApi.list({ unread_only: filter.value === 'unread', page_size: 30 }),
    notificationsApi.settings(),
  ])
  items.value = page.items
  unreadCount.value = page.unread_count
  toastEnabled.value = settings.web_ui.toast_enabled
  demoMode.value = isNotificationsDemoMode()
}

function showToast(item: NotificationItem): void {
  if (!toastEnabled.value || !isToastHost.value) return
  activeToast.value = item
  if (toastTimer) clearTimeout(toastTimer)
  toastTimer = setTimeout(() => {
    if (activeToast.value?.id === item.id) {
      activeToast.value = null
    }
  }, 6000)
}

function dismissToast(): void {
  activeToast.value = null
  if (toastTimer) {
    clearTimeout(toastTimer)
    toastTimer = null
  }
}

async function toggleOpen(): Promise<void> {
  open.value = !open.value
  if (open.value) {
    await refresh()
  }
}

async function setFilter(next: 'all' | 'unread'): Promise<void> {
  filter.value = next
  await refresh()
}

async function openNotification(item: NotificationItem): Promise<void> {
  if (!item.is_read) {
    await notificationsApi.markRead([item.id])
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
  await refresh()
}

async function markAllRead(): Promise<void> {
  await notificationsApi.markAllRead()
  await refresh()
}

async function purgeRead(): Promise<void> {
  await notificationsApi.purgeRead()
  await refresh()
}

async function removeItem(id: string): Promise<void> {
  await notificationsApi.remove(id)
  await refresh()
}

async function simulateDemo(): Promise<void> {
  await notificationsApi.simulateDemoNotification()
  await refresh()
}

function topicLabel(topic: NotificationTopic): string {
  return t(`notifications.topic.${topic}` as MessageKey)
}

function statusLabel(status: OutboxDeliveryStatus): string {
  return t(`notifications.status.${status}` as MessageKey)
}

function severityBadgeClass(severity: NotificationSeverity): string {
  if (severity === 'error') {
    return 'border-rose-500/30 bg-rose-500/10 text-rose-500'
  }
  if (severity === 'warning') {
    return 'border-amber-500/30 bg-amber-500/10 text-amber-500'
  }
  if (severity === 'success') {
    return 'border-emerald-500/30 bg-emerald-500/10 text-emerald-500'
  }
  return 'border-sky-500/30 bg-sky-500/10 text-sky-500'
}

function deliveryStatusClass(status: OutboxDeliveryStatus): string {
  if (status === 'sent') return 'text-emerald-500'
  if (status === 'failed') return 'text-rose-500'
  if (status === 'sending' || status === 'pending') return 'text-amber-500'
  return 'text-[var(--text-muted)]'
}

onMounted(() => {
  mountedBellUids.add(instanceUid)
  isToastHost.value = mountedBellUids.values().next().value === instanceUid
  void refresh()
  unsubscribeEvents = subscribeNotificationEvents({
    onCreated: item => {
      showToast(item)
      void refresh()
    },
    onChanged: () => {
      void refresh()
    },
  })
})

onUnmounted(() => {
  mountedBellUids.delete(instanceUid)
  unsubscribeEvents?.()
  if (toastTimer) clearTimeout(toastTimer)
})
</script>

<template>
  <div class="relative inline-flex items-center">
    <button
      type="button"
      data-testid="notification-bell-trigger"
      class="relative inline-flex items-center justify-center gap-2 rounded-lg border border-[var(--border)] bg-[var(--bg-subtle)] text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-hover)] hover:text-[var(--text)]"
      :class="props.compact ? 'h-9 w-9 p-0' : 'h-9 px-2.5 text-xs font-medium'"
      :aria-label="t('notifications.bellLabel', { count: unreadCount })"
      :title="t('notifications.bellLabel', { count: unreadCount })"
      @click="toggleOpen"
    >
      <Bell class="h-4 w-4 shrink-0" />
      <span v-if="!props.compact" class="truncate">{{ t('notifications.title') }}</span>
      <span
        v-if="unreadCount > 0"
        data-testid="notification-unread-badge"
        class="inline-flex min-w-[18px] items-center justify-center rounded-full bg-rose-600 px-1.5 py-0.5 text-[10px] font-bold leading-none text-white"
        :class="props.compact ? 'absolute -right-1 -top-1' : ''"
      >
        {{ unreadCount > 99 ? '99+' : unreadCount }}
      </span>
    </button>

    <!-- Inbox Popover -->
    <div
      v-if="open"
      data-testid="notification-popover"
      class="absolute z-50 rounded-xl border border-[var(--border)] bg-[var(--bg-elevated)] p-3 shadow-2xl"
      :class="popoverPositionClass"
    >
      <div class="flex items-center justify-between gap-2 border-b border-[var(--border)] pb-2.5">
        <div class="flex items-center gap-2">
          <h3 class="text-sm font-semibold text-[var(--text)]">
            {{ t('notifications.title') }}
          </h3>
          <span
            v-if="demoMode"
            class="rounded-full border border-amber-500/30 bg-amber-500/10 px-2 py-0.5 text-[10px] font-medium text-amber-500"
          >
            {{ t('notifications.demoBadge') }}
          </span>
        </div>

        <div class="flex items-center gap-1">
          <button
            type="button"
            data-testid="notification-simulate-demo"
            class="inline-flex items-center gap-1 rounded-md border border-[var(--border)] bg-[var(--bg-subtle)] px-2 py-1 text-[11px] font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-hover)] hover:text-[var(--text)]"
            @click="simulateDemo"
          >
            <Sparkles class="h-3 w-3 text-amber-500" />
            <span>{{ t('notifications.simulateDemo') }}</span>
          </button>
          <button
            type="button"
            class="inline-flex h-6 w-6 items-center justify-center rounded-md text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text)]"
            :aria-label="t('common.close')"
            @click="open = false"
          >
            <X class="h-3.5 w-3.5" />
          </button>
        </div>
      </div>

      <!-- Filter Tabs & Bulk Actions -->
      <div class="mt-2.5 flex items-center justify-between gap-2">
        <div class="inline-flex rounded-lg border border-[var(--border)] bg-[var(--bg-subtle)] p-0.5 text-xs">
          <button
            type="button"
            data-testid="notification-tab-all"
            class="rounded-md px-2.5 py-1 font-medium transition-colors"
            :class="
              filter === 'all'
                ? 'bg-[var(--bg-elevated)] text-[var(--text)] shadow-xs'
                : 'text-[var(--text-muted)] hover:text-[var(--text)]'
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
                ? 'bg-[var(--bg-elevated)] text-[var(--text)] shadow-xs'
                : 'text-[var(--text-muted)] hover:text-[var(--text)]'
            "
            @click="setFilter('unread')"
          >
            {{ t('notifications.filter.unread') }} ({{ unreadCount }})
          </button>
        </div>

        <div class="flex items-center gap-1.5 text-[11px]">
          <button
            type="button"
            data-testid="notification-mark-all-read"
            class="inline-flex items-center gap-1 rounded-md px-2 py-1 font-medium text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text)] disabled:opacity-40"
            :disabled="unreadCount === 0"
            @click="markAllRead"
          >
            <CheckCheck class="h-3.5 w-3.5" />
            <span>{{ t('notifications.markAllRead') }}</span>
          </button>
          <button
            type="button"
            data-testid="notification-purge-read"
            class="inline-flex items-center gap-1 rounded-md px-2 py-1 font-medium text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-rose-500"
            @click="purgeRead"
          >
            <Trash2 class="h-3.5 w-3.5" />
            <span>{{ t('notifications.purgeRead') }}</span>
          </button>
        </div>
      </div>

      <!-- Notification List -->
      <div class="mt-2.5 max-h-80 space-y-2 overflow-y-auto pr-0.5">
        <p
          v-if="items.length === 0"
          class="py-8 text-center text-xs text-[var(--text-muted)]"
        >
          {{ t('notifications.empty') }}
        </p>

        <div
          v-for="item in items"
          :key="item.id"
          data-testid="notification-item"
          class="group rounded-lg border p-2.5 transition-colors"
          :class="
            item.is_read
              ? 'border-[var(--border)] bg-[var(--bg-subtle)]/50 opacity-75'
              : 'border-[var(--border-strong)] bg-[var(--bg-subtle)]'
          "
        >
          <div class="flex items-start justify-between gap-2">
            <button
              type="button"
              data-testid="notification-open-resource"
              class="min-w-0 flex-1 text-left"
              @click="openNotification(item)"
            >
              <div class="flex flex-wrap items-center gap-1.5">
                <span
                  v-if="!item.is_read"
                  class="h-2 w-2 shrink-0 rounded-full bg-sky-500"
                  :title="t('notifications.unreadDot')"
                />
                <span
                  class="rounded-md border px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide"
                  :class="severityBadgeClass(item.severity)"
                >
                  {{ topicLabel(item.topic) }}
                </span>
              </div>
              <p class="mt-1 text-xs font-semibold text-[var(--text)]">
                {{ item.title }}
              </p>
              <p class="mt-0.5 line-clamp-2 text-[11px] text-[var(--text-secondary)]">
                {{ item.body }}
              </p>
            </button>

            <button
              type="button"
              data-testid="notification-delete-item"
              class="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-[var(--text-muted)] opacity-70 hover:bg-[var(--bg-hover)] hover:text-rose-500 group-hover:opacity-100"
              :aria-label="t('notifications.deleteItem')"
              :title="t('notifications.deleteItem')"
              @click.stop="removeItem(item.id)"
            >
              <X class="h-3.5 w-3.5" />
            </button>
          </div>

          <div
            v-if="item.deliveries.length > 0"
            class="mt-2 flex flex-wrap items-center gap-2 border-t border-[var(--border)]/60 pt-1.5 text-[10px]"
          >
            <span
              v-for="delivery in item.deliveries"
              :key="delivery.channel"
              class="inline-flex items-center gap-1 font-medium"
              :class="deliveryStatusClass(delivery.status)"
              :title="delivery.last_error_code ?? undefined"
            >
              <span class="uppercase">{{ delivery.channel }}:</span>
              <span>{{ statusLabel(delivery.status) }}</span>
            </span>
          </div>
        </div>
      </div>
    </div>

    <!-- Live Toast Popup -->
    <div
      v-if="activeToast"
      data-testid="notification-toast"
      class="fixed bottom-4 right-4 z-50 flex w-80 items-start justify-between gap-2 rounded-xl border border-[var(--border-strong)] bg-[var(--bg-elevated)] p-3 shadow-2xl"
      role="status"
      aria-live="polite"
    >
      <div class="min-w-0 flex-1">
        <span
          class="inline-block rounded-md border px-1.5 py-0.5 text-[10px] font-semibold uppercase"
          :class="severityBadgeClass(activeToast.severity)"
        >
          {{ topicLabel(activeToast.topic) }}
        </span>
        <p class="mt-1 text-xs font-semibold text-[var(--text)]">
          {{ activeToast.title }}
        </p>
        <p class="mt-0.5 line-clamp-2 text-[11px] text-[var(--text-secondary)]">
          {{ activeToast.body }}
        </p>
      </div>
      <button
        type="button"
        class="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text)]"
        :aria-label="t('notifications.toastDismiss')"
        @click="dismissToast"
      >
        <X class="h-3.5 w-3.5" />
      </button>
    </div>
  </div>
</template>

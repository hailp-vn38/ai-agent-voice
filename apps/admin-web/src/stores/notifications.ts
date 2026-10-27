import { createPinia, defineStore, getActivePinia, setActivePinia } from 'pinia'
import { ref } from 'vue'

import {
  isNotificationsDemoMode,
  listenNotificationStream,
  notificationsApi,
  onResetNotificationsForTest,
} from '@/api/notifications'
import type { NotificationItem, NotificationSettings } from '@/api/types/notifications'

const useRawNotificationsStore = defineStore('notifications', () => {
  const filter = ref<'all' | 'unread'>('all')
  const items = ref<NotificationItem[]>([])
  const unreadCount = ref(0)
  const toastEnabled = ref(true)
  const activeToast = ref<NotificationItem | null>(null)
  const demoMode = ref(false)
  const toastHostUid = ref<number | null>(null)

  const mountedBellUids = new Set<number>()
  let settingsRequested = false
  let streamAbort: AbortController | null = null
  let toastTimer: ReturnType<typeof setTimeout> | null = null

  function updateToastHost(): void {
    toastHostUid.value = mountedBellUids.values().next().value ?? null
  }

  function showToast(item: NotificationItem): void {
    if (!toastEnabled.value || toastHostUid.value === null) return
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

  async function refresh(options: { includeSettings?: boolean } = {}): Promise<void> {
    const shouldFetchSettings = Boolean(options.includeSettings) || !settingsRequested
    if (shouldFetchSettings) {
      settingsRequested = true
    }
    const listPromise = notificationsApi.list({
      unread_only: filter.value === 'unread',
      page_size: 30,
    })
    const settingsPromise = shouldFetchSettings ? notificationsApi.settings() : null
    try {
      const [page, settings] = await Promise.all([listPromise, settingsPromise])
      items.value = page.items
      unreadCount.value = page.unread_count
      if (settings) {
        toastEnabled.value = settings.web_ui.toast_enabled
      }
      demoMode.value = isNotificationsDemoMode()
    } catch (err) {
      if (shouldFetchSettings) {
        settingsRequested = false
      }
      throw err
    }
  }

  async function setFilter(next: 'all' | 'unread'): Promise<void> {
    filter.value = next
    await refresh()
  }

  async function markRead(ids: string[]): Promise<void> {
    await notificationsApi.markRead(ids)
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

  function startStreamIfNeeded(): void {
    if (streamAbort) return
    streamAbort = new AbortController()
    void listenNotificationStream(
      {
        onCreated: (item: NotificationItem) => {
          showToast(item)
          void refresh()
        },
        onChanged: () => {
          void refresh()
        },
        onSettingsUpdated: (settings: NotificationSettings) => {
          toastEnabled.value = settings.web_ui.toast_enabled
          settingsRequested = true
          demoMode.value = isNotificationsDemoMode()
        },
      },
      streamAbort.signal,
    )
  }

  function stopStream(): void {
    streamAbort?.abort()
    streamAbort = null
  }

  function registerBell(uid: number): void {
    const wasEmpty = mountedBellUids.size === 0
    mountedBellUids.add(uid)
    updateToastHost()
    if (wasEmpty) {
      void refresh({ includeSettings: !settingsRequested })
      startStreamIfNeeded()
    }
  }

  function unregisterBell(uid: number): void {
    mountedBellUids.delete(uid)
    updateToastHost()
    if (mountedBellUids.size === 0) {
      stopStream()
      dismissToast()
      settingsRequested = false
    }
  }

  function resetState(): void {
    mountedBellUids.clear()
    toastHostUid.value = null
    stopStream()
    dismissToast()
    filter.value = 'all'
    items.value = []
    unreadCount.value = 0
    toastEnabled.value = true
    demoMode.value = false
    settingsRequested = false
  }

  onResetNotificationsForTest(resetState)

  return {
    filter,
    items,
    unreadCount,
    toastEnabled,
    activeToast,
    demoMode,
    toastHostUid,
    refresh,
    setFilter,
    markRead,
    markAllRead,
    purgeRead,
    removeItem,
    simulateDemo,
    dismissToast,
    registerBell,
    unregisterBell,
    resetState,
  }
})

export function useNotificationsStore() {
  if (!getActivePinia()) {
    setActivePinia(createPinia())
  }
  return useRawNotificationsStore()
}

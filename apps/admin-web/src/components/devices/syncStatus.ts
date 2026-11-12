/** Label key for a reminder's sync status; unknown or missing values keep the generic awaiting-sync label. */
export function syncLabelKey(syncStatus: string | undefined): 'reminders.syncPending' | 'reminders.syncWakeOnDue' {
  return syncStatus === 'wake_on_due' ? 'reminders.syncWakeOnDue' : 'reminders.syncPending'
}

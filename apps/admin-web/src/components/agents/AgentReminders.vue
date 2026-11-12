<script setup lang="ts">
import { ref, watch, onBeforeUnmount } from 'vue'
import { Bell, CalendarClock, CheckCircle2, Clock, Hourglass, XCircle } from '@lucide/vue'
import { remindersApi, type AgentReminderPage } from '@/api/reminders'
import { formatApiError } from '@/api/errors'
import DeviceReminders from '@/components/devices/DeviceReminders.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{ agentId: string }>()
const { t, formatDateTime } = useI18n()
const data = ref<AgentReminderPage>()
const error = ref('')
const loading = ref(false)
const device = ref('')
const status = ref('')
const from = ref('')
const to = ref('')
const page = ref(1)
const devicePage = ref(1)
const target = ref('')
let active: AbortController | undefined
const reminderStates = [
  { key: 'pending', icon: Clock }, { key: 'completed', icon: CheckCircle2 },
  { key: 'cancelled', icon: XCircle }, { key: 'expired', icon: Hourglass },
] as const

async function load() {
  active?.abort()
  const controller = new AbortController()
  active = controller
  loading.value = true
  error.value = ''
  const fromMs = from.value ? Date.parse(from.value) : undefined
  const toMs = to.value ? Date.parse(to.value) : undefined
  const explicit = (value: string) => !value || (/(?:Z|[+-]\d{2}:\d{2})$/.test(value) && Number.isFinite(Date.parse(value)))
  if (!explicit(from.value) || !explicit(to.value) || (fromMs !== undefined && toMs !== undefined && fromMs > toMs)) {
    error.value = t('reminders.timeFilterError')
    loading.value = false
    return
  }
  try {
    const result = await remindersApi.listAgent(props.agentId, {
      device_id: device.value || undefined,
      status: status.value || undefined,
      due_from_ms: fromMs,
      due_to_ms: toMs,
      page: page.value,
      device_page: devicePage.value,
    }, controller.signal)
    if (!controller.signal.aborted) data.value = result
  } catch (cause) {
    if (!controller.signal.aborted) error.value = formatApiError(cause)
  } finally {
    if (active === controller) loading.value = false
  }
}
function filter() {
  page.value = 1
  void load()
}
function pickerPage(delta: number) {
  devicePage.value += delta
  target.value = ''
  device.value = ''
  page.value = 1
  void load()
}
function manage(id: string) { target.value = id }
watch(() => props.agentId, () => {
  data.value = undefined
  target.value = ''
  device.value = ''
  status.value = ''
  from.value = ''
  to.value = ''
  page.value = 1
  devicePage.value = 1
  void load()
}, { immediate: true })
onBeforeUnmount(() => active?.abort())
</script>

<template>
  <section class="body-card body-stack">
    <header class="body-section-head">
      <div class="body-heading-copy">
        <span class="body-section-icon" aria-hidden="true"><CalendarClock :size="17" /></span>
        <div><h2>{{ t('reminders.title') }}</h2><p>{{ t('reminders.kpiHelp') }}</p></div>
      </div>
      <Button size="sm" variant="outline" :disabled="loading" @click="load">{{ t('common.refresh') }}</Button>
    </header>

    <dl v-if="data && !loading && !error" class="reminder-kpis">
      <div v-for="state in reminderStates" :key="state.key" :data-kpi="state.key" class="reminder-kpi">
        <dt><component :is="state.icon" :size="14" aria-hidden="true" />{{ t(`reminders.${state.key}`) }}</dt>
        <dd>{{ data.kpi[state.key] }}</dd>
      </div>
    </dl>
    <p v-if="error" role="alert" class="body-alert">{{ error }}</p>
    <div v-if="loading" role="status" aria-busy="true" class="space-y-3">
      <span class="sr-only">{{ t('common.loading') }}</span>
      <div class="reminder-kpis" aria-hidden="true">
        <div v-for="slot in 4" :key="slot" class="card-shimmer space-y-2 rounded-xl border border-border p-4">
          <div class="h-3 w-20 rounded bg-muted/50" /><div class="h-6 w-12 rounded bg-muted/60" />
        </div>
      </div>
      <div v-for="row in 2" :key="row" class="card-shimmer space-y-2 rounded-xl border border-border p-4" aria-hidden="true">
        <div class="h-4 w-2/3 rounded bg-muted/60" /><div class="h-3 w-1/3 rounded bg-muted/40" />
      </div>
    </div>

    <form class="reminder-filters" @submit.prevent="filter">
      <label>{{ t('reminders.device') }}
        <select v-model="device" data-filter="device" class="admin-input" @change="filter">
          <option value="">{{ t('reminders.all') }}</option>
          <option v-for="item in data?.devices" :key="item.device_id" :value="item.device_id">{{ item.name || item.device_id }}</option>
        </select>
      </label>
      <label>{{ t('reminders.filter') }}
        <select v-model="status" class="admin-input" @change="filter">
          <option value="">{{ t('reminders.all') }}</option>
          <option v-for="state in reminderStates" :key="state.key" :value="state.key">{{ t(`reminders.${state.key}`) }}</option>
        </select>
      </label>
      <label>{{ t('reminders.from') }}<input v-model="from" class="admin-input" placeholder="2030-01-01T00:00:00Z" /></label>
      <label>{{ t('reminders.to') }}<input v-model="to" class="admin-input" placeholder="2030-01-31T00:00:00Z" /></label>
      <Button type="submit" size="sm" :disabled="loading">{{ t('reminders.apply') }}</Button>
    </form>
    <p class="body-caption">{{ t('reminders.timestamp') }}</p>
    <div class="reminder-pagination">
      <Button size="sm" variant="ghost" :disabled="devicePage === 1 || loading" @click="pickerPage(-1)">{{ t('reminders.previousDevices') }}</Button>
      <span>{{ devicePage }}</span>
      <Button size="sm" variant="ghost" :disabled="!data?.device_has_more || loading" @click="pickerPage(1)">{{ t('reminders.nextDevices') }}</Button>
    </div>

    <div v-if="data && !data.items.length && !loading && !error" class="body-empty"><Bell :size="30" aria-hidden="true" /><p>{{ t('reminders.empty') }}</p></div>
    <ul v-if="!loading && !error" class="reminder-list">
      <li v-for="item in data?.items" :key="item.id" class="reminder-item">
        <Bell :size="17" aria-hidden="true" />
        <div class="reminder-item-main">
          <p>{{ item.message }}</p>
          <div class="reminder-item-meta">
            <RouterLink :to="{ name: 'device-detail', params: { deviceId: item.device_id } }">{{ item.device_id }}</RouterLink>
            <time>{{ formatDateTime(new Date(item.due_at_ms)) }}</time>
            <span class="reminder-state" :data-state="item.status">{{ t(`reminders.${item.status}` as Parameters<typeof t>[0]) }}</span>
          </div>
          <p class="body-caption mt-2">{{ t('reminders.syncPending') }}</p>
        </div>
        <Button data-manage size="sm" variant="outline" @click="manage(item.device_id)">{{ t('reminders.manage') }}</Button>
      </li>
    </ul>
    <div class="reminder-pagination">
      <Button size="sm" variant="outline" :disabled="page === 1 || loading" @click="page--; load()">{{ t('reminders.previous') }}</Button>
      <span>{{ page }} · {{ data?.total ?? 0 }}</span>
      <Button size="sm" variant="outline" :disabled="!data?.has_more || loading" @click="page++; load()">{{ t('reminders.next') }}</Button>
    </div>

    <div class="reminder-manage">
      <label class="reminder-manage-picker">{{ t('reminders.manageDevice') }}
        <select v-model="target" class="admin-input">
          <option value="">{{ t('reminders.chooseDevice') }}</option>
          <option v-for="item in data?.devices" :key="item.device_id" :value="item.device_id">{{ item.name || item.device_id }}</option>
        </select>
      </label>
      <div v-if="target" class="reminder-device-editor"><DeviceReminders :key="`${agentId}:${target}`" :device-id="target" :agent-id="agentId" @changed="load" /></div>
    </div>
  </section>
</template>

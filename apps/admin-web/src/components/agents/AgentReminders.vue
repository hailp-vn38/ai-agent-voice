<script setup lang="ts">
import { ref, watch, onBeforeUnmount } from 'vue'
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
 <section class="space-y-5">
  <h2 class="text-lg font-semibold">{{t('reminders.title')}}</h2>
  <p>{{t('reminders.kpiHelp')}}</p>
  <dl v-if="data && !loading && !error" class="grid grid-cols-2 gap-3 sm:grid-cols-4"><div v-for="state in ['pending','completed','cancelled','expired'] as const" :key="state" :data-kpi="state" class="rounded border p-3"><dt>{{t(`reminders.${state}`)}}</dt><dd>{{data.kpi[state]}}</dd></div></dl>
  <p v-if="error" role="alert">{{error}}</p>
  <div v-if="loading" role="status" aria-busy="true" class="space-y-3">
   <span class="sr-only">{{t('common.loading')}}</span>
   <div class="grid grid-cols-2 gap-3 sm:grid-cols-4" aria-hidden="true">
    <div v-for="slot in 4" :key="slot" class="card-shimmer space-y-2 rounded border p-3">
     <div class="h-3 w-20 rounded bg-muted/50" />
     <div class="h-6 w-12 rounded bg-muted/60" />
    </div>
   </div>
   <div v-for="row in 2" :key="row" class="card-shimmer space-y-2 rounded border p-3" aria-hidden="true">
    <div class="h-4 w-2/3 rounded bg-muted/60" />
    <div class="h-3 w-1/3 rounded bg-muted/40" />
   </div>
  </div>
  <form class="flex flex-wrap items-end gap-3" @submit.prevent="filter">
   <label>{{t('reminders.device')}}<select v-model="device" data-filter="device" class="block rounded border py-2 pl-2.5 pr-9" @change="filter"><option value="">{{t('reminders.all')}}</option><option v-for="item in data?.devices" :key="item.device_id" :value="item.device_id">{{item.name || item.device_id}}</option></select></label>
   <label>{{t('reminders.filter')}}<select v-model="status" class="block rounded border py-2 pl-2.5 pr-9" @change="filter"><option value="">{{t('reminders.all')}}</option><option v-for="state in ['pending','completed','cancelled','expired'] as const" :key="state" :value="state">{{t(`reminders.${state}`)}}</option></select></label>
   <label>{{t('reminders.from')}}<input v-model="from" class="block rounded border p-2" placeholder="2030-01-01T00:00:00Z" /></label><label>{{t('reminders.to')}}<input v-model="to" class="block rounded border p-2" placeholder="2030-01-31T00:00:00Z" /></label><Button type="submit" :disabled="loading">{{t('reminders.apply')}}</Button>
  </form>
  <p>{{t('reminders.timestamp')}}</p>
  <div class="flex gap-2"><Button :disabled="devicePage===1 || loading" @click="pickerPage(-1)">{{t('reminders.previousDevices')}}</Button><span>{{devicePage}}</span><Button :disabled="!data?.device_has_more || loading" @click="pickerPage(1)">{{t('reminders.nextDevices')}}</Button></div>
  <p v-if="data && !data.items.length && !loading">{{t('reminders.empty')}}</p>
  <ul v-if="!loading && !error" class="space-y-3"><li v-for="item in data?.items" :key="item.id" class="rounded border p-3"><p>{{item.message}}</p><RouterLink :to="{name:'device-detail',params:{deviceId:item.device_id}}" class="underline">{{item.device_id}}</RouterLink><time class="block">{{formatDateTime(new Date(item.due_at_ms))}}</time><p>{{t(`reminders.${item.status}` as Parameters<typeof t>[0])}} · {{t('reminders.syncPending')}}</p><Button data-manage @click="manage(item.device_id)">{{t('reminders.manage')}}</Button></li></ul>
  <div class="flex items-center gap-3"><Button :disabled="page===1 || loading" @click="page--;load()">{{t('reminders.previous')}}</Button><span>{{page}} · {{data?.total ?? 0}}</span><Button :disabled="!data?.has_more || loading" @click="page++;load()">{{t('reminders.next')}}</Button><Button :disabled="loading" @click="load">{{t('common.refresh')}}</Button></div>
  <label>{{t('reminders.manageDevice')}}<select v-model="target" class="block rounded border py-2 pl-2.5 pr-9"><option value="">{{t('reminders.chooseDevice')}}</option><option v-for="item in data?.devices" :key="item.device_id" :value="item.device_id">{{item.name || item.device_id}}</option></select></label>
  <DeviceReminders v-if="target" :key="`${agentId}:${target}`" :device-id="target" :agent-id="agentId" @changed="load" />
 </section>
</template>

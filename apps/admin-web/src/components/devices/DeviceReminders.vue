<script setup lang="ts">
import { ref, watch, onBeforeUnmount } from 'vue'
import { remindersApi, type ReminderInput, type ReminderPage } from '@/api/reminders'
import { formatApiError } from '@/api/errors'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{ deviceId: string }>()
const { t, formatDateTime } = useI18n()
const result = ref<ReminderPage>()
const error = ref('')
const saved = ref(false)
const loading = ref(false)
const saving = ref(false)
const message = ref('')
const delay = ref(600)
const at = ref('')
const mode = ref('relative')
const status = ref('pending')
const page = ref(1)
let active: AbortController | undefined
let mutation: { payload: string; key: string } | undefined

async function load() {
  active?.abort()
  const controller = new AbortController()
  active = controller
  loading.value = true
  error.value = ''
  try {
    const data = await remindersApi.list(props.deviceId, status.value, page.value, controller.signal)
    if (!controller.signal.aborted) result.value = data
  } catch (cause) {
    if (!controller.signal.aborted) error.value = formatApiError(cause)
  } finally {
    if (active === controller) loading.value = false
  }
}

async function create() {
  if (saving.value) return
  const input: ReminderInput = mode.value === 'relative'
    ? { message: message.value, delay_seconds: delay.value }
    : { message: message.value, scheduled_at: at.value }
  const target = props.deviceId
  const payload = JSON.stringify(input)
  if (mutation?.payload !== payload) mutation = { payload, key: crypto.randomUUID() }
  saving.value = true
  saved.value = false
  error.value = ''
  try {
    await remindersApi.create(target, input, mutation.key)
    if (props.deviceId !== target) return
    mutation = undefined
    message.value = ''
    saved.value = true
    page.value = 1
    await load()
  } catch (cause) {
    if (props.deviceId === target) error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

watch(() => props.deviceId, () => {
  result.value = undefined
  mutation = undefined
  saved.value = false
  page.value = 1
  void load()
}, { immediate: true })
watch([status, page], () => void load())
onBeforeUnmount(() => active?.abort())
</script>

<template>
 <section class="space-y-4 rounded-xl border p-5" aria-labelledby="reminders-title">
  <h2 id="reminders-title" class="text-lg font-semibold">{{t('reminders.title')}}</h2>
  <p>{{t('reminders.syncHelp')}}</p>
  <p v-if="result?.schedule_summary.reminder_capable===false">{{t('reminders.unsupported')}}</p>
  <p v-if="saved" role="status">{{t('reminders.saved')}}</p>
  <p v-if="error" role="alert">{{error}}</p>
  <form class="space-y-3" @submit.prevent="create">
   <label class="block">{{t('reminders.message')}}<textarea v-model="message" class="block w-full rounded border p-2" required /></label>
   <label class="block">{{t('reminders.timeMode')}}<select v-model="mode" class="block rounded border p-2"><option value="relative">{{t('reminders.relative')}}</option><option value="absolute">{{t('reminders.absolute')}}</option></select></label>
   <label v-if="mode==='relative'" class="block">{{t('reminders.seconds')}}<input v-model.number="delay" class="block rounded border p-2" type="number" required min="1" max="31536000" /></label>
   <label v-else class="block">{{t('reminders.timestamp')}}<input v-model="at" class="block w-full rounded border p-2" required placeholder="2026-10-13T07:00:00+07:00" /></label>
   <Button type="submit" :disabled="saving || !message.trim()">{{t('common.save')}}</Button>
  </form>
  <label>{{t('reminders.filter')}}<select v-model="status" @change="page=1"><option value="">{{t('reminders.all')}}</option><option v-for="state in ['pending','completed','cancelled','expired']" :key="state" :value="state">{{t(`reminders.${state}` as Parameters<typeof t>[0])}}</option></select></label>
  <p v-if="loading" role="status">{{t('common.loading')}}</p>
  <p v-if="!loading && result?.items.length===0">{{t('reminders.empty')}}</p>
  <ul v-if="!loading" class="space-y-3"><li v-for="item in result?.items" :key="item.id" class="rounded border p-3"><p>{{item.message}}</p><time>{{formatDateTime(new Date(item.due_at_ms))}}</time><p>{{t(`reminders.${item.status}` as Parameters<typeof t>[0])}} · {{t('reminders.syncPending')}}</p></li></ul>
  <div class="flex gap-2"><Button :disabled="page===1 || loading" @click="page--">{{t('reminders.previous')}}</Button><span>{{page}}</span><Button :disabled="!result || !result.has_more || loading" @click="page++">{{t('reminders.next')}}</Button><Button :disabled="loading" @click="load">{{t('common.refresh')}}</Button></div>
 </section>
</template>

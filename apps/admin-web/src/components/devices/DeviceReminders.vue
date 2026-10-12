<script setup lang="ts">
import { ref, computed, watch, onBeforeUnmount } from 'vue'
import { remindersApi, type ReminderInput, type ReminderPage, type Reminder } from '@/api/reminders'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import { formatApiError, isApiError } from '@/api/errors'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{ deviceId: string; agentId?: string }>()
const emit = defineEmits<{ changed: [] }>()
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
const editing = ref<Reminder>()
const cancelling = ref<Reminder>()
const cancelOpen = ref(false)
let cancelMutation: { id: string; revision: number; key: string } | undefined
const validTime = computed(() => mode.value === 'relative'
  ? Number.isInteger(delay.value) && delay.value >= 1 && delay.value <= 31536000
  : /(?:Z|[+-]\d{2}:\d{2})$/.test(at.value) && Number.isFinite(Date.parse(at.value)))

function edit(item: Reminder) {
  editing.value = item
  message.value = item.message
  mode.value = 'absolute'
  at.value = new Date(item.due_at_ms).toISOString()
  mutation = undefined
  saved.value = false
  error.value = ''
}
function closeEdit() {
  editing.value = undefined
  message.value = ''
  mutation = undefined
}
function confirmCancel(item: Reminder) {
  cancelling.value = item
  cancelOpen.value = true
}
async function cancelReminder() {
  const item = cancelling.value
  if (!item || saving.value) return
  const target = props.deviceId
  const agent = props.agentId
  if (cancelMutation?.id !== item.id || cancelMutation.revision !== item.row_revision) {
    cancelMutation = { id: item.id, revision: item.row_revision, key: crypto.randomUUID() }
  }
  saving.value = true
  saved.value = false
  error.value = ''
  try {
    await remindersApi.cancel(target, item.id, item.row_revision, cancelMutation.key, agent)
    if (props.deviceId !== target || props.agentId !== agent) return
    cancelMutation = undefined
    await load()
    emit('changed')
  } catch (cause) {
    if (props.deviceId === target && props.agentId === agent) {
      await load()
      error.value = isApiError(cause) && cause.code === 'revision_conflict'
        ? t('reminders.conflict') : formatApiError(cause)
    }
  } finally {
    saving.value = false
  }
}
let active: AbortController | undefined
let mutation: { payload: string; key: string } | undefined

async function load() {
  active?.abort()
  const controller = new AbortController()
  active = controller
  loading.value = true
  error.value = ''
  try {
    const data = await remindersApi.list(props.deviceId, status.value, page.value, controller.signal, props.agentId)
    if (!controller.signal.aborted) result.value = data
  } catch (cause) {
    if (!controller.signal.aborted) error.value = formatApiError(cause)
  } finally {
    if (active === controller) loading.value = false
  }
}

async function create() {
  if (saving.value || !validTime.value) return
  const input: ReminderInput = mode.value === 'relative'
    ? { message: message.value, delay_seconds: delay.value }
    : { message: message.value, scheduled_at: at.value }
  const target = props.deviceId
  const agent = props.agentId
  const item = editing.value
  if (item) input.timezone = item.timezone
  const payload = JSON.stringify([item?.id, item?.row_revision, input])
  if (mutation?.payload !== payload) mutation = { payload, key: crypto.randomUUID() }
  saving.value = true
  saved.value = false
  error.value = ''
  try {
    if (item) await remindersApi.update(target, item.id, item.row_revision, input, mutation.key, agent)
    else await remindersApi.create(target, input, mutation.key, agent)
    if (props.deviceId !== target || props.agentId !== agent) return
    mutation = undefined
    editing.value = undefined
    message.value = ''
    saved.value = true
    page.value = 1
    await load()
    emit('changed')
  } catch (cause) {
    if (props.deviceId === target && props.agentId === agent) {
      if (item && isApiError(cause) && cause.code === 'revision_conflict') {
        await load()
        closeEdit()
        error.value = t('reminders.conflict')
      }
      else error.value = formatApiError(cause)
    }
  } finally {
    saving.value = false
  }
}

watch(() => [props.deviceId, props.agentId], () => {
  result.value = undefined
  editing.value = undefined
  cancelling.value = undefined
  cancelOpen.value = false
  cancelMutation = undefined
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
   <Button type="submit" :disabled="saving || !message.trim() || !validTime">{{t('common.save')}}</Button>
   <Button v-if="editing" type="button" :disabled="saving" @click="closeEdit">{{t('common.cancel')}}</Button>
  </form>
  <label>{{t('reminders.filter')}}<select v-model="status" @change="page=1"><option value="">{{t('reminders.all')}}</option><option v-for="state in ['pending','completed','cancelled','expired']" :key="state" :value="state">{{t(`reminders.${state}` as Parameters<typeof t>[0])}}</option></select></label>
  <p v-if="loading" role="status">{{t('common.loading')}}</p>
  <p v-if="!loading && result?.items.length===0">{{t('reminders.empty')}}</p>
  <ul v-if="!loading" class="space-y-3"><li v-for="item in result?.items" :key="item.id" class="rounded border p-3"><p>{{item.message}}</p><time>{{formatDateTime(new Date(item.due_at_ms))}}</time><p>{{t(`reminders.${item.status}` as Parameters<typeof t>[0])}} · {{t('reminders.syncPending')}}</p><div v-if="item.status==='pending'" class="flex gap-2"><Button data-action="edit" :disabled="saving" @click="edit(item)">{{t('reminders.edit')}}</Button><Button data-action="cancel-reminder" :disabled="saving" @click="confirmCancel(item)">{{t('reminders.cancel')}}</Button></div></li></ul>
  <div class="flex gap-2"><Button :disabled="page===1 || loading" @click="page--">{{t('reminders.previous')}}</Button><span>{{page}}</span><Button :disabled="!result || !result.has_more || loading" @click="page++">{{t('reminders.next')}}</Button><Button :disabled="loading" @click="load">{{t('common.refresh')}}</Button></div>
 <ConfirmDialog v-model="cancelOpen" :title="t('reminders.cancelTitle')" :description="t('reminders.cancelHelp')" :confirm-label="t('reminders.cancel')" @confirm="cancelReminder" />
 </section>
</template>

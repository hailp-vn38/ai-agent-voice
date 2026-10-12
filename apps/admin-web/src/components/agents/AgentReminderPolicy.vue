<script setup lang="ts">
import { ref, watch } from 'vue'
import { remindersApi, type ReminderPolicy } from '@/api/reminders'
import { formatApiError } from '@/api/errors'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{ agentId: string }>()
const { t } = useI18n()
const policy = ref<ReminderPolicy>()
const error = ref('')
const busy = ref(false)
let generation = 0

watch(() => props.agentId, async () => {
  const current = ++generation
  policy.value = undefined
  error.value = ''
  try {
    const data = await remindersApi.policy(props.agentId)
    if (current === generation) policy.value = data
  } catch (cause) {
    if (current === generation) error.value = formatApiError(cause)
  }
}, { immediate: true })

async function change(event: Event) {
  if (!policy.value || busy.value) return
  busy.value = true
  error.value = ''
  const current = generation
  const checkbox = event.target as HTMLInputElement
  const enabled = checkbox.checked
  checkbox.checked = policy.value.reminders_enabled
  try {
    const data = await remindersApi.setPolicy(props.agentId, enabled, policy.value.revision)
    if (current === generation) policy.value = data
  } catch (cause) {
    if (current === generation) error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <section class="space-y-3 rounded-xl border p-5">
    <h2 class="text-lg font-semibold">{{ t('reminders.permission') }}</h2>
    <p>{{ t('reminders.permissionHelp') }}</p>
    <p v-if="error" role="alert">{{ error }}</p>
    <p v-if="!policy && !error" role="status">{{ t('common.loading') }}</p>
    <label>
      <input type="checkbox" :checked="policy?.reminders_enabled ?? false" :disabled="!policy || busy" @change="change" />
      {{ t('reminders.allow') }}
    </label>
  </section>
</template>

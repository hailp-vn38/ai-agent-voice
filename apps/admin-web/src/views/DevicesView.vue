<script setup lang="ts">
import { Info, MonitorSmartphone, Plus, RefreshCw, Search, ShieldCheck, ShieldOff, X } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import CardListSkeleton from '@/components/admin/CardListSkeleton.vue'
import DirectoryHero from '@/components/admin/DirectoryHero.vue'
import { formatApiError } from '@/api/errors'
import { devicesApi } from '@/api/devices'
import type { AdminAgent, AgentTemplateLink } from '@/api/types/agents'
import type { AdminDevice, ClaimDeviceEnrollmentInput } from '@/api/types/devices'
import ClaimDeviceEnrollmentModal from '@/components/agents/ClaimDeviceEnrollmentModal.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import BaseModal from '@/components/admin/BaseModal.vue'
import DeviceCard from '@/components/devices/DeviceCard.vue'
import { asDevice } from '@/components/devices/presentation'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useErrorToast } from '@/composables/useNotify'
import { useAdminStore } from '@/stores/admin'

const store = useAdminStore()
const router = useRouter()
const { t } = useI18n()
const search = ref('')
const agentFilter = ref('')
const admissionFilter = ref<'all' | 'enabled' | 'disabled'>('all')
const rawDevices = ref<AdminDevice[]>([])
const loading = ref(false)
const saving = ref(false)
const error = ref('')
useErrorToast(() => error.value, { retry: () => load() })
const chooseAgentOpen = ref(false)
const enrollmentOpen = ref(false)
const selectedAgentKey = ref('')
const deleteTarget = ref<AdminDevice>()
let activeRequest: AbortController | undefined

const sortedAgents = computed(() => [...store.agents].sort((a, b) => a.name.localeCompare(b.name)))
const selectedAgent = computed(() => store.getAgent(selectedAgentKey.value))
const claimAgent = computed<AdminAgent>(() => ({
  key: selectedAgent.value?.id ?? '',
  name: selectedAgent.value?.name ?? '',
  description: selectedAgent.value?.description ?? null,
  enabled: true,
  revision: store.revisions.agents[selectedAgentKey.value] ?? 0,
}))
const claimTemplates = computed<AgentTemplateLink[]>(() =>
  store.getTemplatesForAgent(selectedAgentKey.value).map((template) => ({
    key: template.id,
    name: template.name,
    language: template.language,
    enabled: true,
    is_default: selectedAgent.value?.defaultTemplateId === template.id,
  })),
)
const devices = computed(() => rawDevices.value.map(asDevice))
const permitted = computed(() => rawDevices.value.filter((device) => device.enabled !== 0).length)
const filtered = computed(() => {
  const needle = search.value.trim().toLocaleLowerCase()
  return devices.value.filter((device) =>
    (!agentFilter.value || device.agentId === agentFilter.value) &&
    (admissionFilter.value === 'all' ||
      (device.status === 'online') === (admissionFilter.value === 'enabled')) &&
    (!needle || [device.name, device.deviceId, device.agentId, device.description]
      .some((part) => part.toLocaleLowerCase().includes(needle))),
  )
})
const heroStats = computed(() => [
  { label: t('deviceDetail.registered'), value: rawDevices.value.length, icon: MonitorSmartphone },
  { label: t('deviceDetail.permittedCount'), value: permitted.value, icon: ShieldCheck },
  { label: t('deviceCard.blocked'), value: rawDevices.value.length - permitted.value, icon: ShieldOff },
])
const admissionOptions = computed(() => [
  { value: 'all' as const, label: t('common.all'), count: rawDevices.value.length, tone: '' },
  { value: 'enabled' as const, label: t('deviceCard.allowed'), count: permitted.value, tone: 'on' },
  { value: 'disabled' as const, label: t('deviceCard.blocked'), count: rawDevices.value.length - permitted.value, tone: 'off' },
])

const hasFilters = computed(() => Boolean(search.value.trim() || agentFilter.value || admissionFilter.value !== 'all'))
function clearFilters() { search.value = ''; agentFilter.value = ''; admissionFilter.value = 'all' }

async function load() {
  activeRequest?.abort()
  const controller = new AbortController()
  activeRequest = controller
  loading.value = true
  error.value = ''
  try {
    const all: AdminDevice[] = []
    // The backend has no server-side text search and does not return total.
    // Fetch every bounded page before applying filters so results are not silently truncated.
    const pageSize = 100
    let completed = false
    for (let page = 1; page <= 100; page += 1) {
      const response = await devicesApi.list({ page, pageSize, sort: 'name' }, controller.signal)
      if (controller.signal.aborted) return
      all.push(...response.items)
      if (response.items.length < pageSize) { completed = true; break }
    }
    if (!completed) throw new Error(t('deviceDetail.listLimit'))
    if (!controller.signal.aborted) rawDevices.value = all
  } catch (cause) {
    if (!controller.signal.aborted) error.value = formatApiError(cause)
  } finally {
    if (activeRequest === controller) {
      loading.value = false
      activeRequest = undefined
    }
  }
}

function startEnrollment() {
  selectedAgentKey.value = sortedAgents.value[0]?.id ?? ''
  chooseAgentOpen.value = true
}

function nextEnrollment() {
  if (!selectedAgent.value) return
  chooseAgentOpen.value = false
  enrollmentOpen.value = true
}

async function claimDevice(input: ClaimDeviceEnrollmentInput) {
  if (saving.value) return
  saving.value = true
  error.value = ''
  try {
    const claimed = await store.claimDeviceEnrollment(input)
    enrollmentOpen.value = false
    await load()
    if (claimed) await router.push({ name: 'device-detail', params: { deviceId: claimed.deviceId } })
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

async function deleteDevice() {
  const target = deleteTarget.value
  if (!target || saving.value) return
  saving.value = true
  error.value = ''
  try {
    await devicesApi.remove(target.device_id, target.revision)
    deleteTarget.value = undefined
    await load()
    void store.refreshAll()
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

function openEdit(deviceId: string) {
  void router.push({ name: 'device-detail', params: { deviceId }, query: { edit: '1' } })
}

onMounted(() => { void load() })
onBeforeUnmount(() => activeRequest?.abort())
</script>

<template>
  <section class="studio-directory" data-directory="devices">
    <DirectoryHero tone="devices" :eyebrow="t('directoryUi.devicesEyebrow')" :title="t('devices.title')" :intro="t('directoryUi.devicesIntro')" :stats="heroStats">
      <template #actions>
        <button type="button" class="hero-primary" @click="startEnrollment"><Plus :size="16" aria-hidden="true" />{{ t('agentDevices.add') }}</button>
        <button type="button" class="hero-ghost" :disabled="loading" @click="load"><RefreshCw :size="16" :class="{ 'animate-spin': loading }" aria-hidden="true" />{{ t('common.refresh') }}</button>
      </template>
    </DirectoryHero>

    <div class="directory-toolbar">
      <label class="directory-search"><span class="sr-only">{{ t('devices.search') }}</span><Search :size="16" aria-hidden="true" /><input v-model="search" type="search" class="admin-input" :placeholder="t('devices.search')" data-device-search /></label>
      <div class="directory-chips" role="group" :aria-label="t('devices.admission')">
        <button v-for="option in admissionOptions" :key="option.value" type="button" class="directory-chip" :aria-pressed="admissionFilter === option.value" @click="admissionFilter = option.value">
          <i v-if="option.tone" :data-tone="option.tone" aria-hidden="true" />{{ option.label }}<b>{{ option.count }}</b>
        </button>
      </div>
      <select v-model="agentFilter" class="admin-input" :aria-label="t('deviceDetail.filterAgent')"><option value="">{{ t('deviceDetail.allAgents') }}</option><option v-for="agent in sortedAgents" :key="agent.id" :value="agent.id">{{ agent.name }}</option></select>
      <button v-if="hasFilters" type="button" class="directory-clear" @click="clearFilters"><X :size="14" aria-hidden="true" />{{ t('providers.clearFilters') }}</button>
      <span v-if="!loading" class="directory-count">{{ t('providers.resultCount', { count: filtered.length }) }}</span>
    </div>

    <CardListSkeleton v-if="loading" class="sm:grid-cols-2 xl:grid-cols-3" />
    <div v-else-if="!filtered.length" class="directory-empty text-center">
      <span class="directory-empty-icon"><MonitorSmartphone :size="28" :stroke-width="1.6" aria-hidden="true" /></span>
      <p>{{ hasFilters ? t('deviceDetail.emptyFilter') : t('devices.empty') }}</p>
      <Button v-if="hasFilters" variant="outline" @click="clearFilters"><X class="size-4" aria-hidden="true" />{{ t('providers.clearFilters') }}</Button>
      <Button v-else @click="startEnrollment"><Plus class="size-4" aria-hidden="true" />{{ t('agentDevices.add') }}</Button>
    </div>
    <ul v-else class="directory-grid" data-devices-grid>
      <DeviceCard
        v-for="device in filtered"
        :key="device.id"
        :device="device"
        :effective-template="store.getEffectiveDeviceTemplate(device)"
        :agent-name="store.getAgent(device.agentId)?.name ?? device.agentId"
        show-agent detail-link
        @edit="openEdit($event.deviceId)"
        @delete="deleteTarget = rawDevices.find((item) => item.device_id === $event.deviceId)"
      />
    </ul>
    <p class="directory-footer-note"><Info :size="14" aria-hidden="true" />{{ t('devices.note') }}</p>

    <BaseModal v-model="chooseAgentOpen" :title="t('deviceDetail.addTitle')" :description="t('deviceDetail.addHint')" width-class="max-w-lg">
      <form class="space-y-4" @submit.prevent="nextEnrollment">
        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('nav.agents') }}</span>
          <select v-model="selectedAgentKey" required class="admin-input">
            <option disabled value="">{{ t('deviceDetail.chooseAgent') }}</option>
            <option v-for="agent in sortedAgents" :key="agent.id" :value="agent.id">{{ agent.name }}</option>
          </select>
        </label>
        <p v-if="!sortedAgents.length" class="text-sm text-muted-foreground">{{ t('deviceDetail.noAgent') }}</p>
        <div class="flex justify-end gap-2">
          <Button type="button" variant="outline" @click="chooseAgentOpen = false">{{ t('common.cancel') }}</Button>
          <Button type="submit" :disabled="!selectedAgent">{{ t('common.next') }}</Button>
        </div>
      </form>
    </BaseModal>
    <ClaimDeviceEnrollmentModal
      v-model="enrollmentOpen"
      :agent="claimAgent"
      :templates="claimTemplates"
      :submitting="saving"
      :error="error"
      @submit="claimDevice"
    />
    <ConfirmDialog
      :model-value="Boolean(deleteTarget)"
      :title="t('agentDevices.deleteTitle', { name: deleteTarget?.name || deleteTarget?.device_id || '' })"
      :description="`${t('agentDevices.deleteDescription')} ${t('memoryV3.deleteWarning')}`"
      :confirm-label="t('agentDevices.delete')"
      tone="danger"
      @update:model-value="(value) => { if (!value) deleteTarget = undefined }"
      @confirm="deleteDevice"
    />
  </section>
</template>

<style scoped src="./studio-directory.css"></style>

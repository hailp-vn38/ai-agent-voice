<script setup lang="ts">
import { MonitorSmartphone, Plus, RefreshCw, Search, ShieldCheck } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { devicesApi } from '@/api/devices'
import type { AdminAgent, AgentTemplateLink } from '@/api/types/agents'
import type { AdminDevice, ClaimDeviceEnrollmentInput } from '@/api/types/devices'
import ClaimDeviceEnrollmentModal from '@/components/agents/ClaimDeviceEnrollmentModal.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import BaseModal from '@/components/admin/BaseModal.vue'
import DeviceCard from '@/components/devices/DeviceCard.vue'
import { asDevice } from '@/components/devices/presentation'
import PageHeader from '@/components/admin/PageHeader.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useAdminStore } from '@/stores/admin'

const store = useAdminStore()
const router = useRouter()
const { t } = useI18n()
const search = ref('')
const agentFilter = ref('')
const admissionFilter = ref('all')
const rawDevices = ref<AdminDevice[]>([])
const loading = ref(false)
const saving = ref(false)
const error = ref('')
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
  <section class="space-y-6">
    <PageHeader :eyebrow="t('devices.eyebrow')" :title="t('devices.title')" :description="t('devices.description')">
      <template #actions>
        <Button variant="outline" :disabled="loading" @click="load">
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" aria-hidden="true" />
          {{ t('common.refresh') }}
        </Button>
        <Button @click="startEnrollment"><Plus class="size-4" aria-hidden="true" />{{ t('agentDevices.add') }}</Button>
      </template>
    </PageHeader>

    <div class="grid grid-cols-2 gap-3" aria-label="Device inventory">
      <div class="studio-panel p-4">
        <p class="flex items-center gap-2 text-xs text-muted-foreground"><MonitorSmartphone class="size-4" aria-hidden="true" />{{ t('deviceDetail.registered') }}</p>
        <p class="mt-2 text-2xl font-semibold tabular-nums">{{ rawDevices.length }}</p>
      </div>
      <div class="studio-panel p-4">
        <p class="flex items-center gap-2 text-xs text-muted-foreground"><ShieldCheck class="size-4" aria-hidden="true" />{{ t('deviceDetail.permittedCount') }}</p>
        <p class="mt-2 text-2xl font-semibold tabular-nums">{{ permitted }}</p>
      </div>
    </div>

    <div class="studio-panel grid gap-3 p-4 md:grid-cols-[minmax(0,1fr)_minmax(9rem,13rem)_minmax(9rem,13rem)]">
      <label class="relative block min-w-0">
        <span class="sr-only">{{ t('devices.search') }}</span>
        <Search class="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
        <input v-model="search" type="search" class="admin-input pl-9" :placeholder="t('devices.search')" data-device-search />
      </label>
      <label>
        <span class="sr-only">{{ t('deviceDetail.filterAgent') }}</span>
        <select v-model="agentFilter" class="admin-input" :aria-label="t('deviceDetail.filterAgent')">
          <option value="">{{ t('deviceDetail.allAgents') }}</option>
          <option v-for="agent in sortedAgents" :key="agent.id" :value="agent.id">{{ agent.name }}</option>
        </select>
      </label>
      <label>
        <span class="sr-only">{{ t('devices.admission') }}</span>
        <select v-model="admissionFilter" class="admin-input" :aria-label="t('devices.admission')">
          <option value="all">{{ t('deviceDetail.allStates') }}</option>
          <option value="enabled">{{ t('deviceCard.allowed') }}</option>
          <option value="disabled">{{ t('deviceCard.blocked') }}</option>
        </select>
      </label>
    </div>

    <div v-if="error" role="alert" class="rounded-lg border border-danger/30 bg-danger/10 p-3 text-sm text-danger-foreground">
      {{ error }}
    </div>
    <div v-if="loading" class="grid gap-3 sm:grid-cols-2 2xl:grid-cols-3" aria-busy="true">
      <div v-for="n in 6" :key="n" class="studio-panel h-64 animate-pulse bg-muted/20" />
    </div>
    <div v-else-if="!filtered.length" class="studio-panel px-5 py-12 text-center">
      <p class="text-sm text-muted-foreground">{{ search || agentFilter || admissionFilter !== 'all' ? t('deviceDetail.emptyFilter') : t('devices.empty') }}</p>
    </div>
    <ul v-else class="grid min-w-0 grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3" data-devices-grid>
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
    <p class="text-xs text-muted-foreground">{{ t('devices.note') }}</p>

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
      :description="t('agentDevices.deleteDescription')"
      :confirm-label="t('agentDevices.delete')"
      tone="danger"
      @update:model-value="(value) => { if (!value) deleteTarget = undefined }"
      @confirm="deleteDevice"
    />
  </section>
</template>

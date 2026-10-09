<script setup lang="ts">
import { Bot, CheckCircle2, ChevronRight, Copy, Layers3, Pencil, RefreshCw, ShieldCheck, ShieldOff, Trash2 } from '@lucide/vue'
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { devicesApi } from '@/api/devices'
import type { AdminDevice } from '@/api/types/devices'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import BaseModal from '@/components/admin/BaseModal.vue'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import VoiceDeviceIcon from '@/components/icons/VoiceDeviceIcon.vue'
import { displayDeviceDate } from '@/components/devices/presentation'
import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useAdminStore } from '@/stores/admin'

const route = useRoute()
const router = useRouter()
const store = useAdminStore()
const { t, formatDateTime } = useI18n()

const deviceId = computed(() => typeof route.params.deviceId === 'string' ? route.params.deviceId : '')
const device = ref<AdminDevice>()
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const editOpen = ref(false)
const deleteOpen = ref(false)
const copied = ref(false)
const form = ref({ name: '', description: '', agentKey: '', templateKey: '' })
let activeRequest: AbortController | undefined

const agent = computed(() => device.value ? store.getAgent(device.value.agent_key) : undefined)
const effectiveTemplate = computed(() => {
  if (!device.value) return undefined
  return device.value.template_key
    ? store.getTemplate(device.value.template_key)
    : store.getDefaultTemplate(device.value.agent_key)
})
const linkedTemplates = computed(() => store.getTemplatesForAgent(form.value.agentKey))
const canSave = computed(() =>
  form.value.agentKey.length > 0 &&
  form.value.name.trim().length > 0 &&
  (form.value.templateKey === '' || linkedTemplates.value.some((item) => item.id === form.value.templateKey)),
)

function formattedDate(seconds?: number) {
  const date = displayDeviceDate(seconds)
  return date ? formatDateTime(date) : t('common.unknown')
}

async function load() {
  activeRequest?.abort()
  const controller = new AbortController()
  activeRequest = controller
  device.value = undefined
  loading.value = true
  error.value = ''
  try {
    if (!deviceId.value) throw new Error(t('deviceDetail.unavailable'))
    const result = await devicesApi.get(deviceId.value, controller.signal)
    if (controller.signal.aborted) return
    device.value = result
    if (route.query.edit === '1') {
      openEdit()
      void router.replace({ name: 'device-detail', params: { deviceId: result.device_id }, query: { ...route.query, edit: undefined } })
    }
  } catch (cause) {
    if (!controller.signal.aborted) error.value = formatApiError(cause)
  } finally {
    if (activeRequest === controller) {
      loading.value = false
      activeRequest = undefined
    }
  }
}

function openEdit() {
  if (!device.value) return
  form.value = {
    name: device.value.name || device.value.device_id,
    description: device.value.description ?? '',
    agentKey: device.value.agent_key,
    templateKey: device.value.template_key ?? '',
  }
  editOpen.value = true
}

async function saveEdit() {
  if (!device.value || !canSave.value || saving.value) return
  saving.value = true
  error.value = ''
  try {
    await devicesApi.update(device.value.device_id, {
      name: form.value.name.trim(),
      description: form.value.description.trim() || null,
      agent_key: form.value.agentKey,
      template_key: form.value.templateKey || null,
    }, device.value.revision)
    editOpen.value = false
    await load()
    void store.refreshAll()
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

async function setAdmission() {
  if (!device.value || saving.value) return
  saving.value = true
  error.value = ''
  try {
    await devicesApi.update(device.value.device_id, { enabled: device.value.enabled === 0 }, device.value.revision)
    await load()
    void store.refreshAll()
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

async function deleteDevice() {
  if (!device.value || saving.value) return
  saving.value = true
  error.value = ''
  try {
    await devicesApi.remove(device.value.device_id, device.value.revision)
    void store.refreshAll()
    await router.push({ name: 'devices' })
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

async function copyId() {
  if (!device.value) return
  try {
    await navigator.clipboard.writeText(device.value.device_id)
    copied.value = true
  } catch {
    error.value = t('deviceCard.copyFailed')
  }
}

function onAgentChanged() {
  // An override cannot silently carry over when switching to a different Agent.
  form.value.templateKey = ''
}

watch(deviceId, () => { editOpen.value = false; deleteOpen.value = false; void load() }, { immediate: true })
onBeforeUnmount(() => activeRequest?.abort())
</script>

<template>
  <section class="space-y-5">
    <div v-if="error" role="alert" class="rounded-lg border border-danger/30 bg-danger/10 px-4 py-3 text-sm text-danger-foreground">
      {{ error }}
      <Button v-if="!device" variant="outline" size="sm" class="ml-2" @click="load">{{ t('common.retry') }}</Button>
    </div>

    <div v-if="loading" class="space-y-4" aria-busy="true">
      <div class="studio-panel h-40 animate-pulse bg-muted/20" />
      <div class="studio-panel h-56 animate-pulse bg-muted/20" />
    </div>
    <div v-else-if="!device" class="studio-panel px-6 py-12 text-center text-sm text-muted-foreground">
      {{ t('deviceDetail.unavailable') }}
    </div>

    <template v-else>
      <DetailHeader :title="device.name || device.device_id" :back-label="t('nav.devices')" @back="router.push({ name: 'devices' })">
        <template #icon>
          <span class="flex size-11 shrink-0 items-center justify-center rounded-xl border border-studio-cyan/20 bg-studio-cyan/10 text-studio-cyan">
            <VoiceDeviceIcon class="size-7" />
          </span>
        </template>
        <template #details>
          <p class="font-medium uppercase tracking-wide text-studio-cyan">{{ t('deviceCard.type') }}</p>
          <p v-if="device.description" class="mt-1 whitespace-pre-wrap break-words text-sm">{{ device.description }}</p>
          <span class="mt-2 inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-medium"
            :class="device.enabled !== 0 ? 'bg-success/10 text-success-foreground' : 'bg-muted text-muted-foreground'">
            <CheckCircle2 v-if="device.enabled !== 0" class="size-3.5" aria-hidden="true" />
            <ShieldOff v-else class="size-3.5" aria-hidden="true" />
            {{ device.enabled !== 0 ? t('deviceCard.allowed') : t('deviceCard.blocked') }}
          </span>
        </template>
        <template #actions>
          <Button variant="outline" :disabled="loading || saving" @click="load">
            <RefreshCw class="size-4" aria-hidden="true" /> {{ t('common.refresh') }}
          </Button>
          <Button :disabled="saving" @click="openEdit"><Pencil class="size-4" aria-hidden="true" />{{ t('agentDevices.edit') }}</Button>
          <ActionMenu :label="t('common.actions')" panel-width="12rem" :disabled="saving">
            <MenuItem variant="danger" @select="deleteOpen = true">
              <Trash2 class="size-4 shrink-0" aria-hidden="true" />
              {{ t('agentDevices.delete') }}
            </MenuItem>
          </ActionMenu>
        </template>
      </DetailHeader>

      <div class="grid items-start gap-4 lg:grid-cols-2">
        <section class="studio-panel min-w-0 p-5">
          <h2 class="font-semibold">{{ t('deviceDetail.identity') }}</h2>
          <dl class="mt-5 grid min-w-0 gap-5 sm:grid-cols-2">
            <div class="min-w-0 sm:col-span-2">
              <dt class="text-xs text-muted-foreground">{{ t('deviceCard.deviceId') }}</dt>
              <dd class="mt-1 flex min-w-0 items-center gap-2">
                <code class="min-w-0 break-all text-sm">{{ device.device_id }}</code>
                <button type="button" data-copy-device-id class="shrink-0 cursor-pointer rounded-md p-2 text-muted-foreground hover:bg-accent hover:text-foreground"
                  :aria-label="t('deviceCard.copyId')" @click="copyId">
                  <CheckCircle2 v-if="copied" class="size-4" aria-hidden="true" />
                  <Copy v-else class="size-4" aria-hidden="true" />
                </button>
              </dd>
            </div>
            <div>
              <dt class="text-xs text-muted-foreground">{{ t('deviceDetail.created') }}</dt>
              <dd class="mt-1 text-sm font-medium">{{ formattedDate(device.created_at) }}</dd>
            </div>
            <div>
              <dt class="text-xs text-muted-foreground">{{ t('deviceDetail.updated') }}</dt>
              <dd class="mt-1 text-sm font-medium">{{ formattedDate(device.updated_at) }}</dd>
            </div>
            <div>
              <dt class="text-xs text-muted-foreground">{{ t('devices.admission') }}</dt>
              <dd class="mt-1 text-sm font-medium">{{ device.enabled !== 0 ? t('devices.enabled') : t('devices.disabled') }}</dd>
            </div>
            <div>
              <dt class="text-xs text-muted-foreground">{{ t('deviceDetail.connection') }}</dt>
              <dd class="mt-1 text-sm text-muted-foreground">{{ t('deviceDetail.notAvailable') }}</dd>
            </div>
          </dl>
        </section>

        <section class="studio-panel min-w-0 p-5">
          <h2 class="font-semibold">{{ t('deviceDetail.aiConfig') }}</h2>
          <div class="mt-5 space-y-4">
            <RouterLink :to="{ name: 'agent-detail', params: { agentId: device.agent_key } }"
              class="flex items-center gap-3 rounded-lg border border-border/70 p-3 hover:bg-surface focus-visible:outline-2 focus-visible:outline-ring">
              <Bot class="size-5 shrink-0 text-studio-violet" aria-hidden="true" />
              <div class="min-w-0 flex-1">
                <p class="text-xs text-muted-foreground">{{ t('nav.agents') }}</p>
                <p class="truncate text-sm font-medium">{{ agent?.name || device.agent_key }}</p>
              </div>
              <ChevronRight class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            </RouterLink>
            <div class="flex items-center gap-3 rounded-lg border border-border/70 p-3">
              <Layers3 class="size-5 shrink-0 text-studio-cyan" aria-hidden="true" />
              <div class="min-w-0 flex-1">
                <p class="text-xs text-muted-foreground">{{ t('deviceDetail.effectiveTemplate') }}</p>
                <p class="break-words text-sm font-medium">{{ effectiveTemplate?.name || device.template_key || t('common.notLinked') }}</p>
                <p class="mt-1 text-xs text-muted-foreground">
                  {{ device.template_key ? t('agentDevices.override') : t('agentDevices.default') }}
                </p>
              </div>
              <RouterLink v-if="effectiveTemplate" :to="{ name: 'template-detail', params: { templateId: effectiveTemplate.id } }"
                :aria-label="t('deviceDetail.openTemplate')" class="rounded-md p-1.5 hover:bg-accent">
                <ChevronRight class="size-4" aria-hidden="true" />
              </RouterLink>
            </div>
          </div>
        </section>
      </div>

      <section class="studio-panel flex flex-wrap items-center justify-between gap-4 p-5">
        <div class="flex min-w-0 items-start gap-3">
          <ShieldCheck class="mt-0.5 size-5 shrink-0 text-studio-violet" aria-hidden="true" />
          <div>
            <h2 class="text-sm font-semibold">{{ t('deviceDetail.access') }}</h2>
            <p class="mt-1 max-w-xl text-xs text-muted-foreground">{{ t('devices.note') }}</p>
          </div>
        </div>
        <button type="button" role="switch" data-device-admission-switch :aria-checked="device.enabled !== 0"
          :aria-label="t('deviceDetail.access')" :disabled="saving"
          class="relative h-7 w-12 shrink-0 cursor-pointer rounded-full transition-colors disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-ring"
          :class="device.enabled !== 0 ? 'bg-success' : 'bg-muted-foreground/40'" @click="setAdmission">
          <span class="absolute top-1 size-5 rounded-full bg-white shadow transition-all" :class="device.enabled !== 0 ? 'left-6' : 'left-1'" />
        </button>
      </section>

    </template>

    <BaseModal v-model="editOpen" :title="t('agentDevices.edit')" width-class="max-w-xl">
      <form id="device-detail-edit-form" class="space-y-4" @submit.prevent="saveEdit">
        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('agents.name') }}</span>
          <input v-model="form.name" class="admin-input" required maxlength="128" />
        </label>
        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('providers.descriptionField') }}</span>
          <textarea v-model="form.description" class="admin-textarea" rows="2" maxlength="2048" />
        </label>
        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('nav.agents') }}</span>
          <select v-model="form.agentKey" class="admin-input" required @change="onAgentChanged">
            <option v-for="item in store.agents" :key="item.id" :value="item.id">{{ item.name }}</option>
          </select>
        </label>
        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('deviceDetail.effectiveTemplate') }}</span>
          <select v-model="form.templateKey" class="admin-input">
            <option value="">{{ t('deviceDetail.followAgent') }}</option>
            <option v-for="item in linkedTemplates" :key="item.id" :value="item.id">{{ item.name }}</option>
          </select>
          <span class="text-xs text-muted-foreground">{{ t('deviceDetail.templateHint') }}</span>
        </label>
      </form>
      <template #footer>
        <div class="flex justify-end gap-2">
          <Button variant="outline" @click="editOpen = false">{{ t('common.cancel') }}</Button>
          <Button type="submit" form="device-detail-edit-form" :disabled="saving || !canSave">{{ t('common.save') }}</Button>
        </div>
      </template>
    </BaseModal>

    <ConfirmDialog v-model="deleteOpen" :title="t('agentDevices.deleteTitle', { name: device?.name || device?.device_id || '' })"
      :description="t('agentDevices.deleteDescription')" :confirm-label="t('agentDevices.delete')" tone="danger" @confirm="deleteDevice" />
  </section>
</template>

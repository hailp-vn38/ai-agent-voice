<script setup lang="ts">
import { Volume2, ArrowDown, ArrowUpRight, BookOpen, Bot, Check, CircleSlash, Fingerprint, GitBranch, LockKeyhole, NotebookPen, CheckCircle2, ChevronRight, Copy, Layers3, Pencil, RefreshCw, ShieldCheck, ShieldOff, Trash2 } from '@lucide/vue'
import { computed, onBeforeUnmount, ref, useId, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { formatApiError } from '@/api/errors'
import DeviceReminders from '@/components/devices/DeviceReminders.vue'
import { devicesApi } from '@/api/devices'
import type { AdminDevice, MemoryMode } from '@/api/types/devices'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import DetailHero from '@/components/admin/DetailHero.vue'
import BaseModal from '@/components/admin/BaseModal.vue'
import VoiceDeviceIcon from '@/components/icons/VoiceDeviceIcon.vue'
import { displayDeviceDate } from '@/components/devices/presentation'
import { ActionMenu, MenuItem } from '@/components/ui/action-menu'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useErrorToast } from '@/composables/useNotify'
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
useErrorToast(() => error.value, { retry: () => load() })
const editOpen = ref(false)
const deleteOpen = ref(false)
const copied = ref(false)
const speechText = ref('')
const speaking = ref(false)
const speechStatus = ref('')
const speechError = ref('')
const form = ref({ name: '', description: '', agentKey: '', templateKey: '', memoryMode: 'off' as MemoryMode })
let activeRequest: AbortController | undefined

const agent = computed(() => device.value ? store.getAgent(device.value.agent_key) : undefined)
const effectiveTemplate = computed(() => {
  if (!device.value) return undefined
  return device.value.template_key
    ? store.getTemplate(device.value.template_key)
    : store.getDefaultTemplate(device.value.agent_key)
})
const presenceLabel = computed(() => device.value?.ws_online === undefined
  ? t('deviceDetail.notAvailable')
  : t(device.value.ws_online ? 'directSpeech.online' : 'directSpeech.offline'),
)
const presenceState = computed(() => device.value?.ws_online === undefined ? 'unknown' : device.value.ws_online ? 'online' : 'offline')

const linkedTemplates = computed(() => store.getTemplatesForAgent(form.value.agentKey))
const memoryChoiceName = useId()
const memoryModes = [
  { value: 'off', icon: CircleSlash, hint: 'deviceEdit.memoryOff' },
  { value: 'read', icon: BookOpen, hint: 'deviceEdit.memoryRead' },
  { value: 'read_write', icon: NotebookPen, hint: 'deviceEdit.memoryWrite' },
] as const
const draftAgent = computed(() => store.getAgent(form.value.agentKey))
const draftTemplate = computed(() => form.value.templateKey
  ? linkedTemplates.value.find((item) => item.id === form.value.templateKey)
  : store.getDefaultTemplate(form.value.agentKey),
)

const canSave = computed(() =>
  form.value.agentKey.length > 0 &&
  form.value.name.trim().length > 0 &&
  (form.value.templateKey === '' || linkedTemplates.value.some((item) => item.id === form.value.templateKey)),
)

async function speak() {
  if (!device.value || speaking.value || !speechText.value.trim()) return
  speaking.value = true
  speechStatus.value = ''
  speechError.value = ''
  try {
    await devicesApi.speak(device.value.device_id, speechText.value, crypto.randomUUID())
    speechStatus.value = t('directSpeech.accepted')
  } catch (cause) {
    speechError.value = formatApiError(cause)
  } finally {
    speaking.value = false
  }
}

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
  if (!device.value || device.value.kind === 'web_assistant') return
  form.value = {
    name: device.value.name || device.value.device_id,
    description: device.value.description ?? '',
    agentKey: device.value.agent_key,
    templateKey: device.value.template_key ?? '',
    memoryMode: device.value.memory_mode,
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
      memory_mode: form.value.memoryMode,
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
  <section class="device-workspace">

    <div v-if="loading" class="space-y-4" aria-busy="true">
      <div class="studio-panel h-40 animate-pulse bg-muted/20" />
      <div class="studio-panel h-56 animate-pulse bg-muted/20" />
    </div>
    <div v-else-if="!device" class="studio-panel px-6 py-12 text-center text-sm text-muted-foreground">
      {{ t('deviceDetail.unavailable') }}
    </div>

    <template v-else>
      <DetailHero tone="devices" :eyebrow="t(device.kind === 'web_assistant' ? 'directoryUi.virtualDevice' : 'directoryUi.deviceWorkspace')" :title="device.name || device.device_id" :description="device.description || undefined" :back-label="t('nav.devices')" @back="router.push({ name: 'devices' })">
        <template #icon><VoiceDeviceIcon class="h-9 w-7" /></template>
        <template #meta>
          <span class="hero-code"><Fingerprint :size="13" aria-hidden="true" /><code>{{ device.device_id }}</code><button type="button" :aria-label="t('deviceCard.copyId')" @click="copyId"><CheckCircle2 v-if="copied" :size="14" aria-hidden="true" /><Copy v-else :size="14" aria-hidden="true" /></button></span>
        </template>
        <template #actions>
          <RouterLink v-if="device.kind === 'web_assistant'" to="/system" class="hero-ghost">{{ t('assistant.managedDevice') }}</RouterLink>
          <button v-else type="button" class="hero-primary" :disabled="saving" @click="openEdit"><Pencil :size="16" aria-hidden="true" />{{ t('agentDevices.edit') }}</button>
          <button type="button" class="hero-ghost" :disabled="loading || saving" @click="load"><RefreshCw :size="16" aria-hidden="true" />{{ t('common.refresh') }}</button>
          <span v-if="device.kind !== 'web_assistant'" class="hero-menu"><ActionMenu :label="t('common.actions')" panel-width="12rem" :disabled="saving"><MenuItem variant="danger" @select="deleteOpen = true"><Trash2 class="size-4" aria-hidden="true" />{{ t('agentDevices.delete') }}</MenuItem></ActionMenu></span>
        </template>
        <template #facts>
          <div class="hero-fact"><span>{{ t('deviceDetail.connection') }}</span><strong :data-presence="presenceState" data-device-presence><i class="device-presence-dot" aria-hidden="true" />{{ presenceLabel }}</strong></div>
          <div class="hero-fact"><span>{{ t('devices.admission') }}</span><strong class="device-admission-label" :data-enabled="device.enabled !== 0"><ShieldCheck v-if="device.enabled !== 0" :size="16" aria-hidden="true" /><ShieldOff v-else :size="16" aria-hidden="true" />{{ device.enabled !== 0 ? t('deviceCard.allowed') : t('deviceCard.blocked') }}</strong></div>
          <div class="hero-fact"><span>{{ t('memoryV3.mode') }}</span><strong><BookOpen :size="16" aria-hidden="true" />{{ t(`memoryV3.mode.${device.memory_mode}`) }}</strong></div>
        </template>
      </DetailHero>
      <div class="device-control-grid"><div class="device-control-primary">
      <section v-if="device.kind !== 'web_assistant'" class="studio-panel device-section device-speech-station">
        <div class="device-section-eyebrow"><span>{{ t('directoryUi.voiceStation') }}</span><Volume2 :size="18" aria-hidden="true" /></div>
        <h2 class="font-semibold">{{ t('directSpeech.title') }}</h2>
        <p class="mt-2 text-sm">{{ presenceLabel }}</p>
        <p v-if="device.ws_online && !device.direct_tts_supported" class="mt-2 text-sm text-muted-foreground">{{ t('directSpeech.unsupported') }}</p>
        <form data-direct-speech class="mt-4 space-y-3" @submit.prevent="speak">
          <label for="direct-speech-text" class="block text-sm">{{ t('directSpeech.text') }}</label>
          <textarea id="direct-speech-text" v-model="speechText" maxlength="1024" rows="3" class="w-full rounded-lg border bg-background p-3" :disabled="speaking" required />
          <Button type="submit" :disabled="speaking || !device.ws_online || !device.direct_tts_supported || !speechText.trim()">
            {{ speaking ? t('common.loading') : t('directSpeech.send') }}
          </Button>
          <p v-if="speechStatus" role="status" class="text-sm">{{ speechStatus }}</p>
          <p v-if="speechError" role="alert" class="text-sm text-danger-foreground">{{ speechError }}</p>
        </form>
      </section>
        <section class="studio-panel device-section device-ai-section">
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
      </div><aside class="device-control-side">
        <section class="studio-panel device-section device-information">
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
              <dt class="text-xs text-muted-foreground">{{ t('memoryV3.mode') }}</dt>
              <dd data-device-memory-mode-value class="mt-1 text-sm font-medium">{{ t(`memoryV3.mode.${device.memory_mode}`) }}</dd>
            </div>
            <div>
              <dt class="text-xs text-muted-foreground">{{ t('deviceDetail.connection') }}</dt>
              <dd class="mt-1 text-sm text-muted-foreground">{{ presenceLabel }}</dd>
            </div>
          </dl>
        </section>
      <section class="studio-panel device-section device-access-section">
        <div class="flex min-w-0 items-start gap-3">
          <ShieldCheck class="mt-0.5 size-5 shrink-0 text-studio-violet" aria-hidden="true" />
          <div>
            <h2 class="text-sm font-semibold">{{ t('deviceDetail.access') }}</h2>
            <p class="mt-1 max-w-xl text-xs text-muted-foreground">{{ t('devices.note') }}</p>
          </div>
        </div>
        <button type="button" role="switch" data-device-admission-switch :aria-checked="device.enabled !== 0"
          :aria-label="t('deviceDetail.access')" :disabled="saving || device.kind === 'web_assistant'"
          class="relative h-7 w-12 shrink-0 cursor-pointer rounded-full transition-colors disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-ring"
          :class="device.enabled !== 0 ? 'bg-success' : 'bg-muted-foreground/40'" @click="setAdmission">
          <span class="absolute top-1 size-5 rounded-full bg-white shadow transition-all" :class="device.enabled !== 0 ? 'left-6' : 'left-1'" />
        </button>
      </section>
      </aside></div>
    </template>

    <BaseModal
      v-model="editOpen"
      :title="t('agentDevices.edit')"
      :description="t('deviceEdit.subtitle')"
      :close-disabled="saving"
      width-class="max-w-[1080px]"
      body-class="p-0"
    >
      <div v-if="device" class="device-edit-studio">
        <aside class="device-edit-passport">
          <div class="device-edit-eyebrow"><span /> DEVICE STUDIO <span>/ 01</span></div>
          <div class="device-edit-portrait" aria-hidden="true">
            <span class="portrait-coordinate coordinate-nw">VOICE / DEVICE</span>
            <svg class="portrait-circuit" viewBox="0 0 240 164" fill="none">
              <path d="M0 43h37l19 19h30M0 121h30l20-20h36M154 62h30l19-19h37M154 101h36l20 20h30" />
              <circle cx="37" cy="43" r="3" /><circle cx="203" cy="43" r="3" />
              <circle cx="30" cy="121" r="3" /><circle cx="210" cy="121" r="3" />
            </svg>
            <VoiceDeviceIcon class="device-edit-device-icon" />
            <span class="portrait-coordinate coordinate-se">{{ t('deviceCard.type') }}</span>
          </div>
          <p class="device-edit-kicker">{{ t('deviceEdit.identity') }}</p>
          <h3 class="device-edit-name">{{ form.name || device.device_id }}</h3>
          <div class="device-edit-hardware-id"><LockKeyhole :size="12" aria-hidden="true" /><code>{{ device.device_id }}</code></div>
          <p class="device-edit-id-hint">{{ t('deviceEdit.idHint') }}</p>
          <div class="device-edit-admission" :data-enabled="device.enabled !== 0">
            <ShieldCheck v-if="device.enabled !== 0" :size="15" aria-hidden="true" /><ShieldOff v-else :size="15" aria-hidden="true" />
            {{ device.enabled !== 0 ? t('devices.enabled') : t('devices.disabled') }}
          </div>

          <div class="device-edit-route">
            <p class="device-edit-kicker">{{ t('deviceEdit.route') }}</p>
            <div class="route-stop"><Bot :size="16" aria-hidden="true" /><div><span>{{ t('nav.agents') }}</span><strong>{{ draftAgent?.name || form.agentKey || t('common.notLinked') }}</strong></div></div>
            <span class="route-connector" aria-hidden="true"><ArrowDown :size="12" /></span>
            <div class="route-stop"><Layers3 :size="16" aria-hidden="true" /><div><span>{{ t('deviceDetail.effectiveTemplate') }}</span><strong>{{ draftTemplate?.name || form.templateKey || t('common.notLinked') }}</strong><small>{{ form.templateKey ? t('agentDevices.override') : t('agentDevices.default') }}</small></div></div>
          </div>
          <p class="device-edit-draft-note">{{ t('deviceEdit.previewHint') }}</p>
        </aside>

        <form id="device-detail-edit-form" class="device-edit-workbench" @submit.prevent="saveEdit">
          <section class="device-edit-section">
            <header class="device-edit-section-heading"><span>01</span><div><h3>{{ t('deviceEdit.identity') }}</h3><p>{{ t('deviceEdit.identityHint') }}</p></div><Fingerprint :size="19" aria-hidden="true" /></header>
            <label class="block space-y-1.5">
              <span class="text-sm font-medium">{{ t('agents.name') }}</span>
              <input v-model="form.name" :disabled="saving" class="admin-input" required maxlength="128" />
            </label>
            <label class="mt-4 block space-y-1.5">
              <span class="text-sm font-medium">{{ t('providers.descriptionField') }} <small class="device-edit-optional">{{ t('deviceEdit.optional') }}</small></span>
              <textarea v-model="form.description" :disabled="saving" class="admin-textarea" rows="2" maxlength="2048" :placeholder="t('deviceEdit.descriptionPlaceholder')" />
            </label>
          </section>

          <section class="device-edit-section">
            <header class="device-edit-section-heading"><span>02</span><div><h3>{{ t('deviceEdit.routing') }}</h3><p>{{ t('deviceEdit.routingHint') }}</p></div><GitBranch :size="19" aria-hidden="true" /></header>
            <div class="grid gap-4 sm:grid-cols-2">
              <label class="block space-y-1.5">
                <span class="text-sm font-medium">{{ t('nav.agents') }}</span>
                <select v-model="form.agentKey" :disabled="saving" class="admin-input" required @change="onAgentChanged">
                  <option v-for="item in store.agents" :key="item.id" :value="item.id">{{ item.name }}</option>
                </select>
              </label>
              <label class="block space-y-1.5">
                <span class="text-sm font-medium">{{ t('deviceDetail.effectiveTemplate') }}</span>
                <select v-model="form.templateKey" :disabled="saving" class="admin-input">
                  <option value="">{{ t('deviceDetail.followAgent') }}</option>
                  <option v-for="item in linkedTemplates" :key="item.id" :value="item.id">{{ item.name }}</option>
                </select>
              </label>
            </div>
            <p class="device-edit-help">{{ t('deviceDetail.templateHint') }}</p>
          </section>

          <section class="device-edit-section device-edit-memory">
            <header class="device-edit-section-heading"><span>03</span><div><h3>{{ t('memoryV3.mode') }}</h3><p>{{ t('deviceEdit.memoryHint') }}</p></div><BookOpen :size="19" aria-hidden="true" /></header>
            <fieldset :disabled="saving" class="device-edit-memory-options">
              <legend class="sr-only">{{ t('memoryV3.mode') }}</legend>
              <label v-for="mode in memoryModes" :key="mode.value" class="device-edit-memory-choice" :class="{ 'is-selected': form.memoryMode === mode.value }">
                <input v-model="form.memoryMode" type="radio" :name="memoryChoiceName" :value="mode.value" data-device-memory-mode />
                <span class="memory-choice-top"><component :is="mode.icon" :size="19" :stroke-width="1.6" aria-hidden="true" /><span class="memory-choice-indicator" aria-hidden="true"><Check v-if="form.memoryMode === mode.value" :size="10" /></span></span>
                <strong>{{ t(`memoryV3.mode.${mode.value}`) }}</strong>
                <span>{{ t(mode.hint) }}</span>
              </label>
            </fieldset>
            <p class="device-edit-help">{{ t('memoryV3.modeHint') }}</p>
            <div class="device-edit-privacy"><ShieldCheck :size="15" aria-hidden="true" /><p>{{ t('memoryV3.privateNotice') }}</p></div>
          </section>
          <p v-if="error" role="alert" class="device-edit-error">{{ error }}</p>
        </form>
      </div>
      <template #footer>
        <div class="device-edit-footer">
          <p><LockKeyhole :size="13" aria-hidden="true" />{{ t('deviceEdit.footerHint') }}</p>
          <div class="flex justify-end gap-2">
            <Button variant="ghost" :disabled="saving" @click="editOpen = false">{{ t('common.cancel') }}</Button>
            <Button type="submit" form="device-detail-edit-form" :disabled="saving || !canSave"><RefreshCw v-if="saving" class="size-4 animate-spin" aria-hidden="true" />{{ t('common.saveChanges') }}<ArrowUpRight v-if="!saving" class="size-4" aria-hidden="true" /></Button>
          </div>
        </div>
      </template>
    </BaseModal>

    <ConfirmDialog v-model="deleteOpen" :title="t('agentDevices.deleteTitle', { name: device?.name || device?.device_id || '' })"
      :description="`${t('agentDevices.deleteDescription')} ${t('memoryV3.deleteWarning')}`" :confirm-label="t('agentDevices.delete')" tone="danger" @confirm="deleteDevice" />
    <DeviceReminders class="device-workspace-reminders" v-if="device?.kind === 'physical'" :device-id="device.device_id" />
  </section>
</template>

<style scoped src="./device-edit-modal.css"></style>

<style scoped src="./device-detail-page.css"></style>

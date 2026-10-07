<script setup lang="ts">
import { ArrowLeft, FileText } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import type { AdminAgent, AgentTemplateLink } from '@/api/types/agents'
import type { ClaimDeviceEnrollmentInput } from '@/api/types/devices'
import AgentToolAllowlist from '@/components/agents/AgentToolAllowlist.vue'
import AgentDeviceList from '@/components/agents/AgentDeviceList.vue'
import ClaimDeviceEnrollmentModal from '@/components/agents/ClaimDeviceEnrollmentModal.vue'
import AgentHeader from '@/components/agents/AgentHeader.vue'
import AgentTemplateSwitcher from '@/components/agents/AgentTemplateSwitcher.vue'
import AiPipeline from '@/components/pipeline/AiPipeline.vue'
import AgentFormModal from '@/components/admin/AgentFormModal.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import DeviceFormModal from '@/components/admin/DeviceFormModal.vue'
import ProviderDetailModal from '@/components/admin/ProviderDetailModal.vue'
import ProviderFormModal from '@/components/admin/ProviderFormModal.vue'
import CopyTemplateDialog from '@/components/templates/CopyTemplateDialog.vue'
import LinkAgentTemplateDialog from '@/components/templates/LinkAgentTemplateDialog.vue'
import LinkTemplateAgentDialog from '@/components/templates/LinkTemplateAgentDialog.vue'
import TemplateConfigurationPanel from '@/components/templates/TemplateConfigurationPanel.vue'
import TemplateFormDialog from '@/components/templates/TemplateFormDialog.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { AgentTemplate, Device, ProviderInstance, ProviderType } from '@/domain/admin'
import { useAdminStore, type TemplateSetupInput } from '@/stores/admin'

const store = useAdminStore()
const route = useRoute()
const router = useRouter()
const { t } = useI18n()

const agentId = computed(() => String(route.params.agentId ?? ''))
const agent = computed(() => store.getAgent(agentId.value))
/** Only templates linked to this agent, never the whole global catalog. */
const templates = computed(() => store.getTemplatesForAgent(agentId.value))
const availableTemplates = computed(() => store.getAvailableTemplatesForAgent(agentId.value))
const devices = computed(() => store.devicesForAgent(agentId.value))

const selectedTemplateId = ref<string | null>(null)
const settingDefaultTemplate = ref(false)
const setDefaultError = ref<string | null>(null)

/** Query param wins, otherwise the agent default; unlinked ids fall back to the default. */
function resolveTemplateId() {
  if (!agent.value) return null
  const fromQuery = typeof route.query.template === 'string' ? route.query.template : undefined
  const candidate = fromQuery ?? agent.value.defaultTemplateId
  const exists = templates.value.some((template) => template.id === candidate)
  const next = exists ? candidate : agent.value.defaultTemplateId
  return next && templates.value.some((template) => template.id === next) ? next : null
}

watch(
  [agent, () => route.query.template],
  () => {
    selectedTemplateId.value = resolveTemplateId()
  },
  { immediate: true },
)

const selectedTemplate = computed(
  () => templates.value.find((template) => template.id === selectedTemplateId.value) ?? undefined,
)
const selectedTemplateAgentCount = computed(() =>
  selectedTemplate.value ? store.getTemplateAgentCount(selectedTemplate.value.id) : 0,
)

/** Names of the other templates binding the same provider instance. */
function sharedTemplates(_type: ProviderType, providerId: string) {
  if (!selectedTemplate.value) return []
  return store
    .templatesUsingProvider(providerId, selectedTemplate.value.id)
    .map((template) => template.name)
}

function selectTemplate(templateId: string) {
  selectedTemplateId.value = templateId
  void router.replace({
    query: { ...route.query, template: templateId },
  })
}

const agentModalOpen = ref(false)
const templateFormOpen = ref(false)
const editingTemplate = ref<AgentTemplate | undefined>()
const copyOpen = ref(false)
const linkTemplateOpen = ref(false)
const linkAgentTemplateOpen = ref(false)
const deviceModalOpen = ref(false)
const editingDevice = ref<Device | undefined>()
const providerDetailOpen = ref(false)
const selectedProvider = ref<ProviderInstance | undefined>()
const providerEditOpen = ref(false)
const editingProvider = ref<ProviderInstance | undefined>()
const deleteAgentOpen = ref(false)
const deleteDeviceTarget = ref<Device | undefined>()
const deleteTemplateTarget = ref<AgentTemplate | undefined>()
const unlinkTemplateTarget = ref<AgentTemplate | undefined>()

const deleteDeviceOpen = computed({
  get: () => Boolean(deleteDeviceTarget.value),
  set: (value: boolean) => {
    if (!value) deleteDeviceTarget.value = undefined
  },
})

const deleteTemplateOpen = computed({
  get: () => Boolean(deleteTemplateTarget.value),
  set: (value: boolean) => {
    if (!value) deleteTemplateTarget.value = undefined
  },
})

const unlinkTemplateOpen = computed({
  get: () => Boolean(unlinkTemplateTarget.value),
  set: (value: boolean) => {
    if (!value) unlinkTemplateTarget.value = undefined
  },
})

const deleteTemplateAgentCount = computed(() =>
  deleteTemplateTarget.value ? store.getTemplateAgentCount(deleteTemplateTarget.value.id) : 0,
)

const deleteTemplateDeviceCount = computed(
  () => (deleteTemplateTarget.value ? store.devicesOverridingTemplate(deleteTemplateTarget.value.id).length : 0),
)

/** A global template only goes away once nothing references it. */
const deleteTemplateBlocked = computed(
  () => deleteTemplateAgentCount.value > 0 || deleteTemplateDeviceCount.value > 0,
)

function openProvider(provider: ProviderInstance) {
  selectedProvider.value = provider
  providerDetailOpen.value = true
}

function editProvider(provider: ProviderInstance) {
  editingProvider.value = provider
  providerDetailOpen.value = false
  providerEditOpen.value = true
}

/** The claim modal is written against the API view, so project the store's models onto it. */
const enrollmentAgent = computed<AdminAgent>(() => ({
  key: agent.value?.id ?? '',
  name: agent.value?.name ?? '',
  description: agent.value?.description ?? null,
  enabled: true,
  revision: store.revisions.agents[agentId.value] ?? 0,
}))

const enrollmentTemplates = computed<AgentTemplateLink[]>(() =>
  templates.value.map((template) => ({
    key: template.id,
    name: template.name,
    language: template.language,
    // The catalog does not track Template enablement, so nothing is filtered out here.
    enabled: true,
    is_default: store.isDefaultTemplateForAgent(template.id, agentId.value),
  })),
)

const enrollmentOpen = ref(false)
const enrollmentSubmitting = ref(false)
const enrollmentError = ref<unknown>(null)

function openAddDevice() {
  enrollmentError.value = null
  enrollmentOpen.value = true
}

async function claimDevice(input: ClaimDeviceEnrollmentInput) {
  enrollmentSubmitting.value = true
  enrollmentError.value = null
  try {
    await store.claimDeviceEnrollment(input)
    enrollmentOpen.value = false
  } catch (cause) {
    enrollmentError.value = cause
  } finally {
    enrollmentSubmitting.value = false
  }
}

function openEditDevice(device: Device) {
  editingDevice.value = device
  deviceModalOpen.value = true
}

async function saveDevice(payload: {
  name: string
  deviceId: string
  description: string
  status: Device['status']
  templateId?: string
}) {
  if (!agent.value || !editingDevice.value) return
  const { templateId, ...fields } = payload
  await store.updateDevice(editingDevice.value.id, { ...fields, templateId })
}

function saveEditedProvider(payload: {
  name: string
  type: ProviderType
  adapter: string
  model: string
  description: string
  status: ProviderInstance['status']
  endpoint?: string
}) {
  if (!editingProvider.value) return
  const { type: _type, ...patch } = payload
  store.updateProvider(editingProvider.value.id, patch)
}

function openAddTemplate() {
  editingTemplate.value = undefined
  templateFormOpen.value = true
}

function openEditTemplate() {
  if (!selectedTemplate.value) return
  editingTemplate.value = selectedTemplate.value
  templateFormOpen.value = true
}

/** Agent-scoped create: the new global template is linked here and becomes default. */
async function saveTemplate(payload: TemplateSetupInput) {
  if (!agent.value) return
  const editing = editingTemplate.value
  const template = await store.saveTemplateFromSetup({ ...payload, id: editing?.id })
  if (!template || editing) return
  await store.linkTemplateToAgent(template.id, agent.value.id)
  await store.setAgentDefaultTemplate(agent.value.id, template.id)
  selectTemplate(template.id)
}

async function linkTemplateToAgent(templateId: string, setAsDefault: boolean) {
  if (!selectedTemplate.value) return
  await store.linkTemplateToAgent(templateId, agent.value?.id ?? '')
  if (setAsDefault) await store.setAgentDefaultTemplate(agent.value?.id ?? '', templateId)
}

/** Agent-scoped link: brings an existing global template into this agent. */
async function linkExistingTemplate(templateId: string, setAsDefault: boolean) {
  if (!agent.value) return
  await store.linkTemplateToAgent(templateId, agent.value.id)
  if (setAsDefault) await store.setAgentDefaultTemplate(agent.value.id, templateId)
  selectTemplate(templateId)
}

function requestUnlinkSelectedTemplateFromAgent() {
  if (selectedTemplate.value) unlinkTemplateTarget.value = selectedTemplate.value
}

async function confirmUnlinkTemplateFromAgent() {
  const target = unlinkTemplateTarget.value
  if (!target || !agent.value) return
  await store.unlinkTemplateFromAgent(target.id, agent.value.id)
  unlinkTemplateTarget.value = undefined
}

function linkTemplateProvider(type: ProviderType, providerId: string) {
  if (!selectedTemplate.value) return
  store.linkProviderToTemplate(selectedTemplate.value.id, type, providerId)
}

function unlinkTemplateProvider(type: ProviderType) {
  if (!selectedTemplate.value) return
  store.unlinkProviderFromTemplate(selectedTemplate.value.id, type)
}

function savePrompt(prompt: string) {
  if (!selectedTemplate.value) return
  store.updateTemplate(selectedTemplate.value.id, { prompt })
}

async function setDefaultTemplate(templateId: string) {
  if (!agent.value) return
  settingDefaultTemplate.value = true
  setDefaultError.value = null
  try {
    const changed = await store.setAgentDefaultTemplate(agent.value.id, templateId)
    if (!changed && store.error) setDefaultError.value = store.error
  } finally {
    settingDefaultTemplate.value = false
  }
}

function viewSelectedTemplate() {
  if (!selectedTemplate.value) return
  void router.push(`/templates/${selectedTemplate.value.id}`)
}

/** A copy is global and unlinked, so it never becomes a template of this agent. */
async function copySelectedTemplate(name: string) {
  if (!selectedTemplate.value) return
  const created = await store.duplicateTemplate(selectedTemplate.value.id, name)
  copyOpen.value = false
  if (created) void router.push(`/templates/${created.id}`)
}

function requestDeleteTemplate() {
  if (selectedTemplate.value) deleteTemplateTarget.value = selectedTemplate.value
}

async function confirmDeleteTemplate() {
  const target = deleteTemplateTarget.value
  if (!target || deleteTemplateBlocked.value) return
  const wasSelected = selectedTemplateId.value === target.id
  const deleted = await store.deleteTemplate(target.id)
  deleteTemplateTarget.value = undefined
  if (!deleted || !wasSelected) return
  const fallback = templates.value.find((template) => template.id !== target.id)?.id
  if (fallback) {
    selectTemplate(fallback)
  } else {
    selectedTemplateId.value = null
    const { template: _template, ...query } = route.query
    void router.replace({ query })
  }
}

async function confirmDeleteAgent() {
  if (!agent.value) return
  if (await store.deleteAgent(agent.value.id)) void router.push('/agents')
}

async function confirmDeleteDevice() {
  if (!deleteDeviceTarget.value) return
  await store.deleteDevice(deleteDeviceTarget.value.id)
  deleteDeviceTarget.value = undefined
}
</script>

<template>
  <div v-if="agent" class="space-y-4 sm:space-y-5">
    <AgentHeader
      :agent="agent"
      :template-count="templates.length"
      :device-count="devices.length"
      @back="router.push('/agents')"
      @add-device="openAddDevice"
      @edit-agent="agentModalOpen = true"
      @delete-agent="deleteAgentOpen = true"
    />

    <template v-if="selectedTemplate">
      <AgentTemplateSwitcher
        :agent="agent"
        :templates="templates"
        :selected-id="selectedTemplateId"
        :provider-count="store.templateProviderCount"
        :setting-default="settingDefaultTemplate"
        @update:selected-id="selectTemplate"
        @create-template="openAddTemplate"
        @link-existing-template="linkAgentTemplateOpen = true"
        @set-default="setDefaultTemplate"
      />
      <p v-if="setDefaultError" role="alert" class="rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700">
        {{ setDefaultError }}
      </p>

      <div class="grid items-start gap-4 lg:grid-cols-[minmax(0,1.35fr)_minmax(320px,0.65fr)]">
        <AiPipeline
          :template="selectedTemplate"
          :providers="store.providers"
          :shared-templates="sharedTemplates"
          @open-provider="openProvider"
          @edit-provider="editProvider"
          @unlink="unlinkTemplateProvider"
          @select="linkTemplateProvider"
        />

        <TemplateConfigurationPanel
          :template="selectedTemplate"
          :provider-count="store.templateProviderCount(selectedTemplate)"
          :agent-count="selectedTemplateAgentCount"
          :device-count="store.devicesUsingTemplate(selectedTemplate.id).length"
          @view-template="viewSelectedTemplate"
          @edit-template="openEditTemplate"
          @link-to-agent="linkTemplateOpen = true"
          @copy-template="copyOpen = true"
          @unlink-from-agent="requestUnlinkSelectedTemplateFromAgent"
          @delete-template="requestDeleteTemplate"
          @save-prompt="savePrompt"
        />
      </div>

      <div
        v-if="selectedTemplateAgentCount > 1"
        class="flex flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border border-border/70 bg-card px-3.5 py-3 text-sm"
      >
        <p class="min-w-0 flex-1 text-muted-foreground">
          {{
            t('agentDetail.sharedNotice', {
              name: selectedTemplate.name,
              count: selectedTemplateAgentCount,
            })
          }}
        </p>
        <Button
          size="sm"
          variant="outline"
          :disabled="agent.defaultTemplateId === selectedTemplate.id"
          @click="requestUnlinkSelectedTemplateFromAgent"
        >
          {{ t('agentDetail.unlink') }}
        </Button>
      </div>
    </template>

    <div
      v-else
      class="rounded-xl border border-dashed border-border/80 bg-card px-6 py-16 text-center"
    >
      <FileText class="mx-auto size-6 text-muted-foreground" aria-hidden="true" />
      <p class="mt-3 text-base font-semibold">{{ t('agentDetail.noTemplates') }}</p>
      <p class="mx-auto mt-1 max-w-md text-sm text-muted-foreground">
        {{ t('agentDetail.noTemplatesDescription') }}
      </p>
      <div class="mt-5 flex flex-wrap justify-center gap-2">
        <Button @click="openAddTemplate">{{ t('agentDetail.createFirst') }}</Button>
        <Button variant="outline" @click="linkAgentTemplateOpen = true">
          {{ t('agentDetail.linkExisting') }}
        </Button>
      </div>
    </div>

    <AgentToolAllowlist :agent-id="agentId" />
    <AgentDeviceList
      :devices="devices"
      :effective-template-by-id="store.getEffectiveDeviceTemplateById"
      @add="openAddDevice"
      @edit="openEditDevice"
      @delete="deleteDeviceTarget = $event"
    />

    <AgentFormModal v-model="agentModalOpen" :agent="agent" @save="store.updateAgent(agent.id, $event)" />
    <TemplateFormDialog
      v-model="templateFormOpen"
      :template="editingTemplate"
      :providers="store.providers"
      :agent-count="selectedTemplateAgentCount"
      :save="saveTemplate"
    />
    <CopyTemplateDialog v-model="copyOpen" :template="selectedTemplate" @copy="copySelectedTemplate" />

    <LinkAgentTemplateDialog
      v-model="linkAgentTemplateOpen"
      :agent="agent"
      :available-templates="availableTemplates"
      @link="linkExistingTemplate"
    />
    <LinkTemplateAgentDialog
      v-model="linkTemplateOpen"
      :template="selectedTemplate"
      :agents="store.agents"
      :linked-agent-ids="store.getAgentsUsingTemplate(selectedTemplate?.id ?? '').map((item) => item.id)"
      @link="linkTemplateToAgent"
    />
    <ClaimDeviceEnrollmentModal
      v-model="enrollmentOpen"
      :agent="enrollmentAgent"
      :templates="enrollmentTemplates"
      :submitting="enrollmentSubmitting"
      :error="enrollmentError"
      @submit="claimDevice"
    />
    <DeviceFormModal
      v-model="deviceModalOpen"
      :device="editingDevice"
      :agent="agent"
      :templates="templates"
      @save="saveDevice"
    />
    <ProviderDetailModal v-model="providerDetailOpen" :provider="selectedProvider" @edit="editProvider" />
    <ProviderFormModal v-model="providerEditOpen" :provider="editingProvider" @save="saveEditedProvider" />

    <ConfirmDialog
      v-model="deleteAgentOpen"
      :title="t('agents.deleteTitle')"
      :confirm-label="t('agents.delete')"
      tone="danger"
      @confirm="confirmDeleteAgent"
    >
      <p>
        {{
          t('agents.deleteDescription', {
            name: agent.name,
            devices: t('count.devices', { count: devices.length }),
            templates: t('count.templates', { count: templates.length }),
          })
        }}
      </p>
    </ConfirmDialog>

    <ConfirmDialog
      v-model="deleteDeviceOpen"
      :title="t('agentDevices.deleteTitle', { name: deleteDeviceTarget?.name ?? '' })"
      :confirm-label="t('agentDevices.delete')"
      tone="danger"
      @confirm="confirmDeleteDevice"
    >
      <p>{{ t('agentDevices.deleteDescription') }}</p>
    </ConfirmDialog>

    <ConfirmDialog
      v-model="deleteTemplateOpen"
      :title="
        deleteTemplateBlocked
          ? t('templateDelete.blockedTitle')
          : t('templateDelete.title', { name: deleteTemplateTarget?.name ?? '' })
      "
      :confirm-label="deleteTemplateBlocked ? t('common.close') : t('templateDelete.submit')"
      :cancel-label="deleteTemplateBlocked ? t('common.close') : t('common.cancel')"
      :tone="deleteTemplateBlocked ? 'default' : 'danger'"
      @confirm="confirmDeleteTemplate"
    >
      <p v-if="deleteTemplateAgentCount > 0">
        {{
          t('templateDelete.agentDetailBlocked', {
            name: deleteTemplateTarget?.name ?? '',
            count: deleteTemplateAgentCount,
          })
        }}
      </p>
      <p v-else-if="deleteTemplateDeviceCount > 0">
        {{ t('templateDelete.blockedDevices', { count: deleteTemplateDeviceCount }) }}
      </p>
      <p v-else>{{ t('templateDelete.cleanAgentDetail') }}</p>
    </ConfirmDialog>

    <ConfirmDialog
      v-model="unlinkTemplateOpen"
      :title="t('templateUnlink.title', { name: unlinkTemplateTarget?.name ?? '' })"
      :confirm-label="t('templateUnlink.submit')"
      tone="danger"
      @confirm="confirmUnlinkTemplateFromAgent"
    >
      <p>{{ t('templateUnlink.description', { name: unlinkTemplateTarget?.name ?? '', agent: agent.name }) }}</p>
    </ConfirmDialog>
  </div>

  <section v-else class="space-y-4 py-16 text-center">
    <h1 class="text-2xl font-semibold">{{ t('agents.notFound') }}</h1>
    <p class="text-sm text-muted-foreground">{{ t('agents.notFoundDescription') }}</p>
    <Button @click="router.push('/agents')">
      <ArrowLeft class="size-4" />
      {{ t('agents.back') }}
    </Button>
  </section>
</template>

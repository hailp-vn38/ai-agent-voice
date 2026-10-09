<script setup lang="ts">
import { Layers3, Pencil } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import ProviderDetailModal from '@/components/admin/ProviderDetailModal.vue'
import ProviderFormModal from '@/components/admin/ProviderFormModal.vue'
import AiPipeline from '@/components/pipeline/AiPipeline.vue'
import CopyTemplateDialog from '@/components/templates/CopyTemplateDialog.vue'
import LinkTemplateAgentDialog from '@/components/templates/LinkTemplateAgentDialog.vue'
import TemplateActionsMenu from '@/components/templates/TemplateActionsMenu.vue'
import TemplateAgentUsageList from '@/components/templates/TemplateAgentUsageList.vue'
import TemplateConfigurationPanel from '@/components/templates/TemplateConfigurationPanel.vue'
import TemplateFormDialog from '@/components/templates/TemplateFormDialog.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { Agent, AgentTemplate, ProviderInstance, ProviderType } from '@/domain/admin'
import { useAdminStore, type TemplateSetupInput } from '@/stores/admin'

const store = useAdminStore()
const route = useRoute()
const router = useRouter()
const { t } = useI18n()

const templateId = computed(() => String(route.params.templateId ?? ''))
const template = computed(() => store.getTemplate(templateId.value))
const agents = computed(() => store.getAgentsUsingTemplate(templateId.value))
const agentCount = computed(() => store.getTemplateAgentCount(templateId.value))

const editOpen = ref(false)
const copyOpen = ref(false)
const linkOpen = ref(false)
const providerDetailOpen = ref(false)
const selectedProvider = ref<ProviderInstance | undefined>()
const providerEditOpen = ref(false)
const editingProvider = ref<ProviderInstance | undefined>()
const deleteTarget = ref<AgentTemplate | undefined>()
const unlinkAgentTarget = ref<Agent | undefined>()
const deleteOpen = computed({
  get: () => Boolean(deleteTarget.value),
  set: (value: boolean) => {
    if (!value) deleteTarget.value = undefined
  },
})
const unlinkOpen = computed({
  get: () => Boolean(unlinkAgentTarget.value),
  set: (value: boolean) => {
    if (!value) unlinkAgentTarget.value = undefined
  },
})

const deleteAgentCount = computed(() =>
  deleteTarget.value ? store.getTemplateAgentCount(deleteTarget.value.id) : 0,
)
const deleteDeviceCount = computed(() =>
  deleteTarget.value ? store.devicesOverridingTemplate(deleteTarget.value.id).length : 0,
)
const deleteBlocked = computed(() => deleteAgentCount.value > 0 || deleteDeviceCount.value > 0)

/** Names of the other templates binding the same provider instance. */
function sharedTemplates(_type: ProviderType, providerId: string) {
  if (!template.value) return []
  return store
    .templatesUsingProvider(providerId, template.value.id)
    .map((other) => other.name)
}

function openProvider(provider: ProviderInstance) {
  selectedProvider.value = provider
  providerDetailOpen.value = true
}

function editProvider(provider: ProviderInstance) {
  editingProvider.value = provider
  providerDetailOpen.value = false
  providerEditOpen.value = true
}

function saveEditedProvider(payload: {
  name: string
  type: ProviderInstance['type']
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

function linkProvider(type: ProviderType, providerId: string) {
  store.linkProviderToTemplate(templateId.value, type, providerId)
}

function unlinkProvider(type: ProviderType) {
  store.unlinkProviderFromTemplate(templateId.value, type)
}

function saveTemplate(input: TemplateSetupInput) {
  return store.saveTemplateFromSetup({ ...input, id: templateId.value })
}

function savePrompt(prompt: string) {
  store.updateTemplate(templateId.value, { prompt })
}

/** From a detail page the copy is the new subject, so open it. */
async function copyTemplate(name: string) {
  const created = await store.duplicateTemplate(templateId.value, name)
  copyOpen.value = false
  if (created) void router.push(`/templates/${created.id}`)
}

function requestDeleteTemplate() {
  deleteTarget.value = template.value
}

async function confirmDeleteTemplate() {
  const target = deleteTarget.value
  if (!target || deleteBlocked.value) return
  const deleted = await store.deleteTemplate(target.id)
  deleteTarget.value = undefined
  if (deleted) void router.push('/templates')
}

async function linkTemplate(agentId: string, setAsDefault: boolean) {
  await store.linkTemplateToAgent(templateId.value, agentId)
  if (setAsDefault) await store.setAgentDefaultTemplate(agentId, templateId.value)
}

function requestUnlinkTemplate(agentId: string) {
  unlinkAgentTarget.value = agents.value.find((agent) => agent.id === agentId)
}

async function confirmUnlinkTemplate() {
  const agent = unlinkAgentTarget.value
  if (!agent) return
  await store.unlinkTemplateFromAgent(templateId.value, agent.id)
  unlinkAgentTarget.value = undefined
}

/** Arrives on the agent with this template already selected. */
function viewAgent(agentId: string) {
  void router.push({ path: `/agents/${agentId}`, query: { template: templateId.value } })
}

function deviceCountByAgent(agentId: string) {
  return store.devicesForAgent(agentId).length
}

function isDefaultForAgent(agentId: string) {
  return store.isDefaultTemplateForAgent(templateId.value, agentId)
}
</script>

<template>
  <div v-if="template" class="space-y-4 sm:space-y-5">
    <DetailHeader :title="template.name" :back-label="t('nav.templates')" @back="router.push('/templates')">
      <template #icon>
        <span class="flex size-11 shrink-0 items-center justify-center rounded-xl border border-studio-violet/20 bg-studio-violet/10 text-studio-violet">
          <Layers3 class="size-6" aria-hidden="true" />
        </span>
      </template>
      <template #details>
        <p>{{ template.language || t('templateCard.noLanguage') }} · {{ t('templates.usedBy', { count: agentCount }) }}</p>
        <p v-if="template.description" class="mt-1 max-w-2xl text-sm leading-relaxed">{{ template.description }}</p>
      </template>
      <template #actions>
          <Button size="sm" variant="outline" @click="editOpen = true">
            <Pencil class="size-4" />
            {{ t('templateConfig.editTemplate') }}
          </Button>
          <TemplateActionsMenu
            :template="template"
            :show-view-details="false"
            @link-to-agent="linkOpen = true"
            @copy="copyOpen = true"
            @delete="requestDeleteTemplate"
          />
      </template>
    </DetailHeader>

    <p v-if="agentCount > 1" class="rounded-lg bg-muted/60 px-3 py-2.5 text-xs leading-relaxed text-muted-foreground">
      {{ t('templates.sharedNotice', { name: template.name, count: agentCount }) }}
    </p>

    <div class="grid items-start gap-4 lg:grid-cols-[minmax(0,1.35fr)_minmax(320px,0.65fr)]">
      <AiPipeline
        :template="template"
        :providers="store.providers"
        :shared-templates="sharedTemplates"
        @open-provider="openProvider"
        @edit-provider="editProvider"
        @unlink="unlinkProvider"
        @select="linkProvider"
      />

      <TemplateConfigurationPanel
        :template="template"
        :provider-count="store.templateProviderCount(template)"
        :agent-count="agentCount"
        :device-count="store.devicesUsingTemplate(template.id).length"
        @view-template="router.push(`/templates/${template.id}`)"
        @edit-template="editOpen = true"
        @link-to-agent="linkOpen = true"
        @copy-template="copyOpen = true"
        @delete-template="requestDeleteTemplate"
        @save-prompt="savePrompt"
      />
    </div>

    <TemplateAgentUsageList
      :agents="agents"
      :device-count-by-agent="deviceCountByAgent"
      :is-default="isDefaultForAgent"
      @view-agent="viewAgent"
      @unlink="requestUnlinkTemplate"
    />

    <TemplateFormDialog
      v-model="editOpen"
      :template="template"
      :providers="store.providers"
      :agent-count="agentCount"
      :save="saveTemplate"
    />
    <CopyTemplateDialog v-model="copyOpen" :template="template" @copy="copyTemplate" />

    <LinkTemplateAgentDialog
      v-model="linkOpen"
      :template="template"
      :agents="store.agents"
      :linked-agent-ids="agents.map((agent) => agent.id)"
      @link="linkTemplate"
    />
    <ProviderDetailModal v-model="providerDetailOpen" :provider="selectedProvider" @edit="editProvider" />
    <ProviderFormModal v-model="providerEditOpen" :provider="editingProvider" @save="saveEditedProvider" />

    <ConfirmDialog
      v-model="deleteOpen"
      :title="
        deleteBlocked
          ? t('templateDelete.blockedTitle')
          : t('templateDelete.title', { name: deleteTarget?.name ?? '' })
      "
      :description="deleteBlocked ? undefined : t('templateDelete.description')"
      :confirm-label="deleteBlocked ? t('common.close') : t('templateDelete.submit')"
      :cancel-label="deleteBlocked ? t('common.close') : t('common.cancel')"
      :tone="deleteBlocked ? 'default' : 'danger'"
      @confirm="confirmDeleteTemplate"
    >
      <p v-if="deleteAgentCount > 0">
        {{ t('templateDelete.blockedAgents', { name: deleteTarget?.name ?? '', count: deleteAgentCount }) }}
      </p>
      <p v-else-if="deleteDeviceCount > 0">
        {{ t('templateDelete.blockedDevices', { count: deleteDeviceCount }) }}
      </p>
      <p v-else>{{ t('templateDelete.clean') }}</p>
    </ConfirmDialog>

    <ConfirmDialog
      v-model="unlinkOpen"
      :title="t('templateUnlink.title', { name: template.name })"
      :confirm-label="t('templateUnlink.submit')"
      tone="danger"
      @confirm="confirmUnlinkTemplate"
    >
      <p>{{ t('templateUnlink.description', { name: template.name, agent: unlinkAgentTarget?.name ?? '' }) }}</p>
    </ConfirmDialog>
  </div>

  <section v-else class="space-y-4 py-16 text-center">
    <h1 class="text-2xl font-semibold">{{ t('templates.notFound') }}</h1>
    <p class="text-sm text-muted-foreground">{{ t('templates.notFoundDescription') }}</p>
    <Button @click="router.push('/templates')">
      <ArrowLeft class="size-4" />
      {{ t('templates.back') }}
    </Button>
  </section>
</template>

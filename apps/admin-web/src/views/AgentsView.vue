<script setup lang="ts">
import { Bot, Layers, MonitorSmartphone, Plus, Search, X } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'

import CardListSkeleton from '@/components/admin/CardListSkeleton.vue'
import DirectoryHero from '@/components/admin/DirectoryHero.vue'
import type { AdminAgent, AgentTemplateLink } from '@/api/types/agents'
import type { ClaimDeviceEnrollmentInput } from '@/api/types/devices'
import ClaimDeviceEnrollmentModal from '@/components/agents/ClaimDeviceEnrollmentModal.vue'
import AgentCard from '@/components/admin/AgentCard.vue'
import AgentFormModal from '@/components/admin/AgentFormModal.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { Agent, ProviderInstance } from '@/domain/admin'
import { useAdminStore } from '@/stores/admin'

const store = useAdminStore()
const router = useRouter()
const { t } = useI18n()

const agentModalOpen = ref(false)
const deviceModalOpen = ref(false)
const deviceAgent = ref<Agent | undefined>()
const enrollmentError = ref<unknown>(null)
const enrollmentSubmitting = ref(false)

const search = ref('')
const agents = computed(() => {
  const query = search.value.trim().toLocaleLowerCase()
  return store.agents.filter((item) => !query || [item.name, item.description, item.id].some((value) => value.toLocaleLowerCase().includes(query)))
})
const linkedTemplateCount = computed(() => store.agents.reduce((sum, item) => sum + store.getTemplatesForAgent(item.id).length, 0))
const linkedDeviceCount = computed(() => store.agents.reduce((sum, item) => sum + store.devicesForAgent(item.id).length, 0))
const heroStats = computed(() => [
  { label: t('agents.title'), value: store.agents.length, icon: Bot },
  { label: t('directoryUi.linkedTemplates'), value: linkedTemplateCount.value, icon: Layers },
  { label: t('directoryUi.linkedDevices'), value: linkedDeviceCount.value, icon: MonitorSmartphone },
])

function defaultTemplateProviders(agent: Agent): ProviderInstance[] {
  const template = store.getDefaultTemplate(agent.id)
  if (!template) return []
  return Object.values(template.providerBindings)
    .map((providerId) => store.getProvider(providerId))
    .filter((provider): provider is ProviderInstance => Boolean(provider))
}

/** The claim modal is written against the API view, so project the store's models onto it. */
const enrollmentAgent = computed<AdminAgent>(() => ({
  key: deviceAgent.value?.id ?? '',
  name: deviceAgent.value?.name ?? '',
  description: deviceAgent.value?.description ?? null,
  enabled: true,
  revision: deviceAgent.value ? store.revisions.agents[deviceAgent.value.id] ?? 0 : 0,
}))

const enrollmentTemplates = computed<AgentTemplateLink[]>(() =>
  !deviceAgent.value
    ? []
    : store.getTemplatesForAgent(deviceAgent.value.id).map((template) => ({
        key: template.id,
        name: template.name,
        language: template.language,
        // The catalog does not track Template enablement, so nothing is filtered out here.
        enabled: true,
        is_default: store.isDefaultTemplateForAgent(template.id, deviceAgent.value!.id),
      })),
)

function addDevice(agent: Agent) {
  deviceAgent.value = agent
  enrollmentError.value = null
  deviceModalOpen.value = true
}

async function claimDevice(input: ClaimDeviceEnrollmentInput) {
  enrollmentSubmitting.value = true
  enrollmentError.value = null
  try {
    await store.claimDeviceEnrollment(input)
    deviceModalOpen.value = false
  } catch (cause) {
    enrollmentError.value = cause
  } finally {
    enrollmentSubmitting.value = false
  }
}

function createAgent(payload: { agent: { name: string; description: string }; templateId: string }) {
  store.createAgent(payload.agent, payload.templateId)
}
</script>

<template>
  <section class="studio-directory" data-directory="agents">
    <DirectoryHero tone="agents" :eyebrow="t('directoryUi.agentsEyebrow')" :title="t('agents.title')" :intro="t('directoryUi.agentsIntro')" :stats="heroStats">
      <template #actions>
        <button type="button" class="hero-primary" @click="agentModalOpen = true"><Plus :size="16" aria-hidden="true" />{{ t('agents.add') }}</button>
      </template>
    </DirectoryHero>

    <div class="directory-toolbar">
      <label class="directory-search"><span class="sr-only">{{ t('directoryUi.searchAgents') }}</span><Search :size="16" aria-hidden="true" /><input v-model="search" type="search" class="admin-input" :placeholder="t('directoryUi.searchAgents')" /></label>
      <button v-if="search" type="button" class="directory-clear" @click="search = ''"><X :size="14" aria-hidden="true" />{{ t('providers.clearFilters') }}</button>
      <span v-if="!store.loading" class="directory-count">{{ t('providers.resultCount', { count: agents.length }) }}</span>
    </div>

    <CardListSkeleton v-if="store.loading" class="sm:grid-cols-2 xl:grid-cols-3" />
    <div v-else-if="agents.length" class="directory-grid">
      <AgentCard
        v-for="agent in agents"
        :key="agent.id"
        :agent="agent"
        :default-template="store.getDefaultTemplate(agent.id)"
        :default-template-providers="defaultTemplateProviders(agent)"
        :template-count="store.getTemplatesForAgent(agent.id).length"
        :device-count="store.devicesForAgent(agent.id).length"
        @open="router.push(`/agents/${agent.id}`)"
        @add-device="addDevice(agent)"
      />
    </div>
    <div v-else class="directory-empty text-center">
      <span class="directory-empty-icon"><Bot :size="28" :stroke-width="1.6" aria-hidden="true" /></span>
      <p>{{ search.trim() ? t('directoryUi.noAgentsMatch') : t('agents.empty') }}</p>
      <Button v-if="search.trim()" variant="outline" @click="search = ''"><X class="size-4" aria-hidden="true" />{{ t('providers.clearFilters') }}</Button>
      <Button v-else @click="agentModalOpen = true"><Plus class="size-4" aria-hidden="true" />{{ t('agents.add') }}</Button>
    </div>

    <AgentFormModal
      v-model="agentModalOpen"
      :templates="store.templates"
      @create-with-template="createAgent"
    />
    <ClaimDeviceEnrollmentModal
      v-if="deviceAgent"
      v-model="deviceModalOpen"
      :agent="enrollmentAgent"
      :templates="enrollmentTemplates"
      :submitting="enrollmentSubmitting"
      :error="enrollmentError"
      @submit="claimDevice"
    />
  </section>
</template>

<style scoped src="./studio-directory.css"></style>

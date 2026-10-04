<script setup lang="ts">
import { Plus } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'

import type { AdminAgent, AgentTemplateLink } from '@/api/types/agents'
import type { ClaimDeviceEnrollmentInput } from '@/api/types/devices'
import ClaimDeviceEnrollmentModal from '@/components/agents/ClaimDeviceEnrollmentModal.vue'
import AgentCard from '@/components/admin/AgentCard.vue'
import AgentFormModal from '@/components/admin/AgentFormModal.vue'
import PageHeader from '@/components/admin/PageHeader.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { Agent, ProviderInstance } from '@/domain/admin'
import { useAdminStore, type AgentTemplateInput } from '@/stores/admin'

const store = useAdminStore()
const router = useRouter()
const { t } = useI18n()

const agentModalOpen = ref(false)
const deviceModalOpen = ref(false)
const deviceAgent = ref<Agent | undefined>()
const enrollmentError = ref<unknown>(null)
const enrollmentSubmitting = ref(false)

const agents = computed(() => store.agents)

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

function createAgent(payload: { agent: { name: string; description: string }; template: AgentTemplateInput }) {
  store.createAgent(payload.agent, payload.template)
}
</script>

<template>
  <section class="space-y-6">
    <PageHeader :eyebrow="t('agents.eyebrow')" :title="t('agents.title')" :description="t('agents.description')">
      <template #actions>
        <Button @click="agentModalOpen = true">
          <Plus class="size-4" />
          {{ t('agents.add') }}
        </Button>
      </template>
    </PageHeader>

    <div class="grid gap-4 lg:grid-cols-2">
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

    <div
      v-if="agents.length === 0"
      class="rounded-xl border border-dashed py-16 text-center text-sm text-muted-foreground"
    >
      {{ t('agents.empty') }}
    </div>

    <AgentFormModal
      v-model="agentModalOpen"
      :providers="store.providers"
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

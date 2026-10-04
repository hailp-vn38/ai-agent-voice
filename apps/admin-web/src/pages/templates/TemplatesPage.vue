<script setup lang="ts">
import { Plus, Search } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'

import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import PageHeader from '@/components/admin/PageHeader.vue'
import CopyTemplateDialog from '@/components/templates/CopyTemplateDialog.vue'
import LinkTemplateAgentDialog from '@/components/templates/LinkTemplateAgentDialog.vue'
import TemplateCard from '@/components/templates/TemplateCard.vue'
import TemplateFormDialog from '@/components/templates/TemplateFormDialog.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { AgentTemplate } from '@/domain/admin'
import type { MessageKey } from '@/i18n/messages'
import { useAdminStore, type TemplateSetupInput } from '@/stores/admin'

const store = useAdminStore()
const router = useRouter()
const { t } = useI18n()

const ALL = 'all'
const IN_USE = 'in-use'

const searchQuery = ref('')
const languageFilter = ref<string>(ALL)
const usageFilter = ref<string>(ALL)

const createOpen = ref(false)
const editingTemplate = ref<AgentTemplate | undefined>()
const formOpen = computed({
  get: () => createOpen.value || Boolean(editingTemplate.value),
  set: (value: boolean) => {
    if (!value) {
      createOpen.value = false
      editingTemplate.value = undefined
    }
  },
})
const linkTarget = ref<AgentTemplate | undefined>()
const linkOpen = computed({
  get: () => Boolean(linkTarget.value),
  set: (value: boolean) => {
    if (!value) linkTarget.value = undefined
  },
})
const copyTarget = ref<AgentTemplate | undefined>()
const copyOpen = computed({
  get: () => Boolean(copyTarget.value),
  set: (value: boolean) => {
    if (!value) copyTarget.value = undefined
  },
})
const deleteTarget = ref<AgentTemplate | undefined>()
const deleteOpen = computed({
  get: () => Boolean(deleteTarget.value),
  set: (value: boolean) => {
    if (!value) deleteTarget.value = undefined
  },
})

const takenTemplateKeys = computed(() => store.templates.map((template) => template.id))

// Derived statistics only: agent usage is read from agentTemplateLinks, never persisted.
const inUseCount = computed(
  () => store.templates.filter((template) => store.getTemplateAgentCount(template.id) > 0).length,
)
const stats = computed<{ key: MessageKey; value: number }[]>(() => [
  { key: 'templates.stat.templates', value: store.templates.length },
  { key: 'templates.stat.inUse', value: inUseCount.value },
  { key: 'templates.stat.unused', value: store.templates.length - inUseCount.value },
  { key: 'templates.stat.links', value: store.agentTemplateLinks.length },
])

const visibleTemplates = computed(() => {
  const query = searchQuery.value.trim().toLowerCase()
  return store.templates.filter((template) => {
    if (languageFilter.value !== ALL && template.language !== languageFilter.value) return false
    const agentCount = store.getTemplateAgentCount(template.id)
    if (usageFilter.value === IN_USE && agentCount === 0) return false
    if (usageFilter.value === 'unused' && agentCount > 0) return false
    if (!query) return true
    return [template.name, template.description, template.language]
      .filter(Boolean)
      .some((field) => field.toLowerCase().includes(query))
  })
})

const deleteAgentCount = computed(() =>
  deleteTarget.value ? store.getTemplateAgentCount(deleteTarget.value.id) : 0,
)
const deleteDeviceCount = computed(() =>
  deleteTarget.value ? store.devicesOverridingTemplate(deleteTarget.value.id).length : 0,
)
const deleteBlocked = computed(() => deleteAgentCount.value > 0 || deleteDeviceCount.value > 0)

function openDetail(template: AgentTemplate) {
  void router.push(`/templates/${template.id}`)
}

function openEdit(template: AgentTemplate) {
  createOpen.value = false
  editingTemplate.value = template
}

function saveTemplate(input: TemplateSetupInput) {
  return store.saveTemplateFromSetup({ ...input, id: editingTemplate.value?.id })
}

/** The copy is a new global template, so the list just shows the new card. */
function copyTemplate(name: string) {
  if (!copyTarget.value) return
  store.duplicateTemplate(copyTarget.value.id, name)
  copyTarget.value = undefined
}

async function confirmDeleteTemplate() {
  if (!deleteTarget.value || deleteBlocked.value) return
  await store.deleteTemplate(deleteTarget.value.id)
  deleteTarget.value = undefined
}

async function linkTemplate(agentId: string, setAsDefault: boolean) {
  if (!linkTarget.value) return
  await store.linkTemplateToAgent(linkTarget.value.id, agentId)
  if (setAsDefault) await store.setAgentDefaultTemplate(agentId, linkTarget.value.id)
  linkTarget.value = undefined
}

function providerNameById(providerId?: string) {
  return store.getProvider(providerId)?.name ?? 'Unknown'
}

function agentNamesFor(templateId: string) {
  return store.getAgentsUsingTemplate(templateId).map((agent) => agent.name)
}
</script>

<template>
  <section class="space-y-6">
    <PageHeader
      :eyebrow="t('templates.eyebrow')"
      :title="t('templates.title')"
      :description="t('templates.description')"
    >
      <template #actions>
        <Button @click="createOpen = true; editingTemplate = undefined">
          <Plus class="size-4" />
          {{ t('templates.create') }}
        </Button>
      </template>
    </PageHeader>

    <dl class="grid grid-cols-2 gap-3 sm:grid-cols-4">
      <div
        v-for="stat in stats"
          :key="stat.key"
        class="rounded-lg border border-border/70 bg-card px-3.5 py-3"
      >
        <dt class="text-xs text-muted-foreground">{{ t(stat.key) }}</dt>
        <dd class="mt-1 text-xl font-semibold tabular-nums">{{ stat.value }}</dd>
      </div>
    </dl>

    <div class="flex flex-col gap-2 sm:flex-row sm:items-center">
      <div class="relative sm:max-w-sm sm:flex-1">
        <Search
          class="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
          aria-hidden="true"
        />
        <input
          v-model="searchQuery"
          class="admin-input pl-9"
          type="search"
          :placeholder="t('templates.search')"
          :aria-label="t('templates.searchLabel')"
        />
      </div>

      <select
        v-model="languageFilter"
        class="admin-input sm:w-44"
        :aria-label="t('templates.languageFilter')"
      >
        <option :value="ALL">{{ t('templates.allLanguages') }}</option>
        <option v-for="language in store.templateLanguages" :key="language" :value="language">
          {{ language }}
        </option>
      </select>

      <select
        v-model="usageFilter"
        class="admin-input sm:w-36"
        :aria-label="t('templates.usageFilter')"
      >
        <option :value="ALL">{{ t('templates.allUsage') }}</option>
        <option :value="IN_USE">{{ t('templates.usage.inUse') }}</option>
        <option value="unused">{{ t('templates.usage.unused') }}</option>
      </select>
    </div>

    <div v-if="visibleTemplates.length" class="grid gap-3 lg:grid-cols-2">
      <TemplateCard
        v-for="template in visibleTemplates"
        :key="template.id"
        :template="template"
        :agent-count="store.getTemplateAgentCount(template.id)"
        :agent-names="agentNamesFor(template.id)"
        :provider-name-by-id="providerNameById"
        @open="openDetail(template)"
        @edit="openEdit(template)"
        @link-to-agent="linkTarget = template"
        @copy="copyTarget = template"
        @delete="deleteTarget = template"
      />
    </div>

    <div
      v-else
      class="rounded-xl border border-dashed border-border/80 py-16 text-center text-sm text-muted-foreground"
    >
      {{ t('templates.empty') }}
    </div>

    <TemplateFormDialog
      v-model="formOpen"
      :template="editingTemplate"
      :providers="store.providers"
      :agent-count="editingTemplate ? store.getTemplateAgentCount(editingTemplate.id) : 0"
      :taken-keys="takenTemplateKeys"
      :save="saveTemplate"
    />

    <CopyTemplateDialog v-model="copyOpen" :template="copyTarget" @copy="copyTemplate" />

    <LinkTemplateAgentDialog
      v-model="linkOpen"
      :template="linkTarget"
      :agents="store.agents"
      :linked-agent-ids="linkTarget ? store.getAgentsUsingTemplate(linkTarget.id).map((agent) => agent.id) : []"
      @link="linkTemplate"
    />

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
  </section>
</template>

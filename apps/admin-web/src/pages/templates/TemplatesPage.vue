<script setup lang="ts">
import { CircleCheck, CircleDashed, Layers3, Link2, Plus, Search, X } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'

import CardListSkeleton from '@/components/admin/CardListSkeleton.vue'
import DirectoryHero from '@/components/admin/DirectoryHero.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import CopyTemplateDialog from '@/components/templates/CopyTemplateDialog.vue'
import LinkTemplateAgentDialog from '@/components/templates/LinkTemplateAgentDialog.vue'
import TemplateCard from '@/components/templates/TemplateCard.vue'
import TemplateFormDialog from '@/components/templates/TemplateFormDialog.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { AgentTemplate } from '@/domain/admin'
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

// Derived statistics only: agent usage is read from agentTemplateLinks, never persisted.
const inUseCount = computed(
  () => store.templates.filter((template) => store.getTemplateAgentCount(template.id) > 0).length,
)
const heroStats = computed(() => [
  { label: t('templates.stat.templates'), value: store.templates.length, icon: Layers3 },
  { label: t('templates.stat.inUse'), value: inUseCount.value, icon: CircleCheck },
  { label: t('templates.stat.unused'), value: store.templates.length - inUseCount.value, icon: CircleDashed },
  { label: t('templates.stat.links'), value: store.agentTemplateLinks.length, icon: Link2 },
])
const usageOptions = computed(() => [
  { value: ALL, label: t('common.all'), count: store.templates.length },
  { value: IN_USE, label: t('templates.usage.inUse'), count: inUseCount.value },
  { value: 'unused', label: t('templates.usage.unused'), count: store.templates.length - inUseCount.value },
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

const hasFilters = computed(() => Boolean(searchQuery.value.trim()) || languageFilter.value !== ALL || usageFilter.value !== ALL)
function clearFilters() {
  searchQuery.value = ''
  languageFilter.value = ALL
  usageFilter.value = ALL
}

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
  <section class="studio-directory" data-directory="templates">
    <DirectoryHero tone="templates" :eyebrow="t('templatesBoard.eyebrow')" :title="t('templates.title')" :intro="t('templatesBoard.description')" :stats="heroStats">
      <template #actions>
        <button type="button" class="hero-primary" @click="createOpen = true; editingTemplate = undefined"><Plus :size="16" aria-hidden="true" />{{ t('templates.create') }}</button>
      </template>
    </DirectoryHero>

    <div class="directory-toolbar">
      <label class="directory-search"><span class="sr-only">{{ t('templates.searchLabel') }}</span><Search :size="16" aria-hidden="true" /><input v-model="searchQuery" class="admin-input" type="search" :placeholder="t('templates.search')" /></label>
      <div class="directory-chips" role="group" :aria-label="t('templates.usageFilter')">
        <button v-for="option in usageOptions" :key="option.value" type="button" class="directory-chip" :aria-pressed="usageFilter === option.value" @click="usageFilter = option.value">{{ option.label }}<b>{{ option.count }}</b></button>
      </div>
      <select v-model="languageFilter" class="admin-input" :aria-label="t('templates.languageFilter')"><option :value="ALL">{{ t('templates.allLanguages') }}</option><option v-for="language in store.templateLanguages" :key="language" :value="language">{{ language }}</option></select>
      <button v-if="hasFilters" type="button" class="directory-clear" @click="clearFilters"><X :size="14" aria-hidden="true" />{{ t('providers.clearFilters') }}</button>
      <span v-if="!store.loading" class="directory-count">{{ t('providers.resultCount', { count: visibleTemplates.length }) }}</span>
    </div>

    <CardListSkeleton v-if="store.loading" class="lg:grid-cols-2" />
    <div v-else-if="visibleTemplates.length" class="directory-grid" data-wide>
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

    <div v-else class="directory-empty text-center">
      <span class="directory-empty-icon"><Layers3 :size="28" :stroke-width="1.6" aria-hidden="true" /></span>
      <p>{{ t('templates.empty') }}</p>
      <Button v-if="hasFilters" variant="outline" @click="clearFilters"><X class="size-4" aria-hidden="true" />{{ t('providers.clearFilters') }}</Button>
      <Button v-else @click="createOpen = true; editingTemplate = undefined"><Plus class="size-4" aria-hidden="true" />{{ t('templates.create') }}</Button>
    </div>

    <TemplateFormDialog
      v-model="formOpen"
      :template="editingTemplate"
      :providers="store.providers"
      :agent-count="editingTemplate ? store.getTemplateAgentCount(editingTemplate.id) : 0"
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

<style scoped src="../../views/studio-directory.css"></style>

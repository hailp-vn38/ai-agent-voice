<script setup lang="ts">
import { Plus, Search } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import type { TemplateProviderType } from '@/api/types/templates'
import CardListSkeleton from '@/components/admin/CardListSkeleton.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import PageHeader from '@/components/admin/PageHeader.vue'
import ProviderFormModal from '@/components/admin/ProviderFormModal.vue'
import LinkProviderTemplateDialog from '@/components/providers/LinkProviderTemplateDialog.vue'
import ProviderCatalogCard from '@/components/providers/ProviderCatalogCard.vue'
import ProviderTypeTabs from '@/components/providers/ProviderTypeTabs.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import {
  llmCapabilityTags,
  providerSupportsLlmTag,
  providerTypes,
  type ProviderInstance,
  type ProviderStatus,
  type ProviderType,
  type ProviderUsageEntry,
} from '@/domain/admin'
import type { MessageKey } from '@/i18n/messages'
import { providerTypeIcons } from '@/lib/providerTypeIcons'
import { useAdminStore } from '@/stores/admin'

const store = useAdminStore()
const router = useRouter()
const route = useRoute()
const { t, providerTypeLabel } = useI18n()

const ALL = 'all'

const SETUP_TYPES: TemplateProviderType[] = ['vad', 'asr', 'llm', 'tts']

function isSetupType(type?: ProviderType): type is TemplateProviderType {
  return SETUP_TYPES.includes(type as TemplateProviderType)
}

const searchQuery = ref('')
const activeType = computed<string>({
  get: () => providerTypes.find((type) => type === route.query.type) ?? ALL,
  set: (type) => {
    void router.replace({ query: { ...route.query, type: type === ALL ? undefined : type } })
  },
})
const statusFilter = ref<string>(ALL)
const tagFilter = ref<string>(ALL)

const linkOpen = ref(false)
const selectedProvider = ref<ProviderInstance | undefined>()
const editingProvider = ref<ProviderInstance | undefined>()
const deleteTarget = ref<ProviderInstance | undefined>()

const deleteOpen = computed({
  get: () => Boolean(deleteTarget.value),
  set: (value: boolean) => {
    if (!value) deleteTarget.value = undefined
  },
})

const isSearching = computed(() => searchQuery.value.trim().length > 0)

/** Editing reuses the legacy form modal; creating goes through the onboarding drawer. */
const formOpen = computed({
  get: () => Boolean(editingProvider.value),
  set: (value: boolean) => {
    if (!value) editingProvider.value = undefined
  },
})

const statusOptions: { value: string; label: MessageKey }[] = [
  { value: ALL, label: 'providers.allStatuses' },
  { value: 'ready', label: 'status.provider.ready' },
  { value: 'disabled', label: 'status.provider.disabled' },
  { value: 'error', label: 'status.provider.error' },
]

/** Usage always comes from the template bindings, never from an agent edge. */
function usageFor(providerId: string): ProviderUsageEntry[] {
  return store.templatesUsingProvider(providerId).map((template) => ({
    templateId: template.id,
    templateName: template.name,
    language: template.language,
    agentNames: store.getAgentsUsingTemplate(template.id).map((agent) => agent.name),
  }))
}

const editingUsageCount = computed(() =>
  editingProvider.value ? store.getProviderTemplateCount(editingProvider.value.id) : 0,
)

const deleteUsage = computed(() => (deleteTarget.value ? usageFor(deleteTarget.value.id) : []))

/** Search and status narrow the catalog first, so the type counts stay truthful. */
const filteredProviders = computed(() => {
  const byStatus = store.searchProviders(searchQuery.value)
  if (statusFilter.value === ALL) return byStatus
  return byStatus.filter((provider) => provider.status === (statusFilter.value as ProviderStatus))
})

const typeCounts = computed(() => {
  const counts: Record<string, number> = { [ALL]: filteredProviders.value.length }
  for (const type of providerTypes) {
    counts[type] = filteredProviders.value.filter((provider) => provider.type === type).length
  }
  return counts
})

const visibleProviders = computed(() => {
  const byType =
    activeType.value === ALL
      ? filteredProviders.value
      : filteredProviders.value.filter((provider) => provider.type === activeType.value)
  if (tagFilter.value === ALL || (activeType.value !== ALL && activeType.value !== 'llm')) {
    return byType
  }
  return byType.filter(
    (provider) => provider.type === 'llm' && providerSupportsLlmTag(provider, tagFilter.value),
  )
})

/** The All view keeps pipeline order so types never interleave in one grid. */
const groups = computed(() =>
  providerTypes
    .map((type) => ({
      key: type as string,
      label: providerTypeLabel(type),
      icon: providerTypeIcons[type],
      providers: visibleProviders.value.filter((provider) => provider.type === type),
    }))
    .filter((group) => group.providers.length > 0),
)

/**
 * A single type view skips the heading, since the active tab already names it.
 */
const displayGroups = computed(() => {
  if (activeType.value !== ALL) {
    return visibleProviders.value.length
      ? [{ key: activeType.value, label: '', icon: undefined, providers: visibleProviders.value }]
      : []
  }
  return groups.value
})

const activeTypeLabel = computed(() =>
  activeType.value === ALL ? t('common.all') : providerTypeLabel(activeType.value as ProviderType),
)

const activeProviderType = computed(() => activeType.value as ProviderType)

function clearFilters() {
  searchQuery.value = ''
  statusFilter.value = ALL
  tagFilter.value = ALL
  activeType.value = ALL
}

/** The API models four provider types, so `vision` never preselects anything. */
function openCreate(type: ProviderType = activeProviderType.value) {
  editingProvider.value = undefined
  const query = isSetupType(type)
    ? { ...route.query, type }
    : { ...route.query, type: undefined }
  void router.push({ name: 'provider-create', query })
}

function openDetail(provider: ProviderInstance) {
  void router.push({ name: 'provider-detail', params: { key: provider.id }, query: route.query })
}

function openEdit(provider: ProviderInstance) {
  if (provider.source === 'config') return
  editingProvider.value = provider
}

function openLink(provider: ProviderInstance) {
  selectedProvider.value = provider
  linkOpen.value = true
}

function saveProvider(payload: {
  name: string
  type: ProviderType
  adapter: string
  model: string
  description: string
  status: ProviderStatus
  endpoint?: string
  apiKey?: string
  configJson?: Record<string, unknown>
}) {
  if (!editingProvider.value || editingProvider.value.source === 'config') return
  // The type of an existing instance is fixed: its bindings already rely on it.
  const { type: _type, ...patch } = payload
  store.updateProvider(editingProvider.value.id, patch)
}

function linkProvider(templateId: string) {
  if (!selectedProvider.value) return
  store.linkProviderToTemplate(templateId, selectedProvider.value.type, selectedProvider.value.id)
}

function confirmDelete() {
  if (!deleteTarget.value || deleteTarget.value.source === 'config') return
  store.deleteProvider(deleteTarget.value.id)
  deleteTarget.value = undefined
}

function providerNameById(id?: string) {
  return store.getProvider(id)?.name ?? t('providerLink.notLinked')
}
</script>

<template>
  <section class="space-y-6">
    <PageHeader :title="t('providers.title')" :description="t('providers.description')">
      <template #actions>
        <Button :aria-label="t('providers.create')" @click="openCreate()">
          <Plus class="size-4" />
          <span class="hidden sm:inline">{{ t('providers.create') }}</span>
        </Button>
      </template>
    </PageHeader>

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
          :placeholder="t('providers.search')"
          :aria-label="t('providers.searchLabel')"
        />
      </div>

      <select
        v-model="statusFilter"
        class="admin-input sm:w-44"
        :aria-label="t('providers.statusFilter')"
      >
        <option v-for="option in statusOptions" :key="option.value" :value="option.value">
          {{ t(option.label) }}
        </option>
      </select>
    </div>

    <ProviderTypeTabs v-model="activeType" :counts="typeCounts" />

    <div
      v-if="activeType === 'llm' || activeType === ALL"
      class="flex flex-wrap items-center gap-1.5"
      role="group"
      aria-label="LLM capability tags"
    >
      <button
        type="button"
        data-tag-filter="all"
        :aria-pressed="tagFilter === ALL"
        :class="[
          'rounded-full border px-2.5 py-1 text-xs font-medium transition-colors',
          tagFilter === ALL
            ? 'border-primary bg-primary text-primary-foreground'
            : 'border-border bg-background text-muted-foreground hover:bg-accent hover:text-foreground',
        ]"
        @click="tagFilter = ALL"
      >
        {{ t('common.all') }}
      </button>
      <button
        v-for="tag in llmCapabilityTags"
        :key="tag"
        type="button"
        :data-tag-filter="tag"
        :aria-pressed="tagFilter === tag"
        :class="[
          'rounded-full border px-2.5 py-1 font-mono text-xs transition-colors',
          tagFilter === tag
            ? 'border-primary bg-primary text-primary-foreground'
            : 'border-border bg-background text-muted-foreground hover:bg-accent hover:text-foreground',
        ]"
        @click="tagFilter = tagFilter === tag ? ALL : tag"
      >
        {{ tag }}
      </button>
    </div>

    <CardListSkeleton v-if="store.loading" class="gap-3 sm:grid-cols-2 xl:grid-cols-3" />
    <div v-else-if="displayGroups.length" class="space-y-8">
      <section v-for="group in displayGroups" :key="group.key" class="space-y-3">
        <h2
          v-if="group.label"
          class="flex items-center gap-2 text-xs font-semibold tracking-wider text-muted-foreground uppercase"
        >
          <component :is="group.icon" class="size-3.5" aria-hidden="true" />
          {{ group.label }}
        </h2>
        <div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
          <ProviderCatalogCard
            v-for="provider in group.providers"
            :key="provider.id"
            :provider="provider"
            :usage="usageFor(provider.id)"
            @open="openDetail(provider)"
            @edit="openEdit(provider)"
            @link="openLink(provider)"
            @duplicate="store.duplicateProvider(provider.id)"
            @delete="deleteTarget = provider"
          />
        </div>
      </section>
    </div>

    <div
      v-else-if="isSearching"
      class="rounded-xl border border-dashed border-border/80 px-4 py-16 text-center"
    >
      <p class="text-sm font-medium">{{ t('providers.searchEmptyTitle', { query: searchQuery.trim() }) }}</p>
      <p class="mt-1 text-sm text-muted-foreground">{{ t('providers.searchEmptyDescription') }}</p>
      <Button variant="outline" class="mt-4" @click="clearFilters">
        {{ t('providers.clearFilters') }}
      </Button>
    </div>

    <div
      v-else-if="activeType !== ALL"
      class="rounded-xl border border-dashed border-border/80 px-4 py-16 text-center"
    >
      <p class="text-sm font-medium">
        {{ t('providers.emptyTypeTitle', { type: activeTypeLabel }) }}
      </p>
      <p class="mt-1 text-sm text-muted-foreground">{{ t('providers.emptyTypeDescription', { type: activeTypeLabel }) }}</p>
      <Button class="mt-4" @click="openCreate(activeProviderType)">
        <Plus class="size-4" />
        {{ t('providers.createForType', { type: activeTypeLabel }) }}
      </Button>
    </div>

    <div
      v-else-if="statusFilter !== ALL"
      class="rounded-xl border border-dashed border-border/80 px-4 py-16 text-center"
    >
      <p class="text-sm font-medium">{{ t('providers.filterEmptyTitle') }}</p>
      <p class="mt-1 text-sm text-muted-foreground">{{ t('providers.filterEmptyDescription') }}</p>
      <Button variant="outline" class="mt-4" @click="clearFilters">
        {{ t('providers.clearFilters') }}
      </Button>
    </div>

    <div v-else class="rounded-xl border border-dashed border-border/80 px-4 py-16 text-center">
      <p class="text-sm font-medium">{{ t('providers.emptyTitle') }}</p>
      <p class="mt-1 text-sm text-muted-foreground">{{ t('providers.emptyDescription') }}</p>
      <Button class="mt-4" @click="openCreate()">
        <Plus class="size-4" />
        {{ t('providers.create') }}
      </Button>
    </div>

    <ProviderFormModal
      v-model="formOpen"
      :provider="editingProvider"
      :providers="store.providers"
      :usage-count="editingUsageCount"
      @save="saveProvider"
    />

    <LinkProviderTemplateDialog
      v-model="linkOpen"
      :provider="selectedProvider"
      :templates="store.templates"
      :provider-name-by-id="providerNameById"
      @link="linkProvider"
    />

    <ConfirmDialog
      v-model="deleteOpen"
      tone="danger"
      :title="t('providerDelete.title', { name: deleteTarget?.name ?? '' })"
      :description="t('providerDelete.description')"
      :confirm-label="t('providerDelete.submit')"
      @confirm="confirmDelete"
    >
      <template v-if="deleteUsage.length">
        <p class="font-medium text-foreground">
          {{ t('providerDelete.inUse', { count: t('count.templates', { count: deleteUsage.length }) }) }}
        </p>
        <ul class="mt-2 space-y-0.5">
          <li v-for="entry in deleteUsage.slice(0, 3)" :key="entry.templateId">
            {{ entry.templateName }}
          </li>
          <li v-if="deleteUsage.length > 3" class="text-muted-foreground">
            {{ t('count.more', { count: deleteUsage.length - 3 }) }}
          </li>
        </ul>
        <p class="mt-2">{{ t('providerDelete.bindingImpact') }}</p>
        <p class="mt-2 text-xs">{{ t('providerDelete.templateKept') }}</p>
      </template>
      <template v-else>{{ t('providerDelete.unused') }}</template>
    </ConfirmDialog>
  </section>
</template>

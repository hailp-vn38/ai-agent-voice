<script setup lang="ts">
import { ArrowRight, BookOpen, Plus, RefreshCw, Search, Trash2 } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { agentJobsApi } from '@/api/agent-jobs'
import { formatApiError, isApiError } from '@/api/errors'
import type {
  BackgroundSettings,
  JobSkillDetail,
  JobSkillSummary,
} from '@/api/types/agent-jobs'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useErrorToast, useSuccessToast } from '@/composables/useNotify'
import JobSkillEditorModal from './JobSkillEditorModal.vue'

const router = useRouter()
const { t } = useI18n()

const settings = ref<BackgroundSettings | null>(null)
const skillFilter = ref<'all' | 'builtin' | 'custom'>('all')
const search = ref('')
const loading = ref(true)
const saving = ref(false)
const error = ref('')
useErrorToast(() => error.value, { title: () => (unavailable.value ? t('jobUi.backendPending') : t('jobUi.loadFailed')), retry: () => load() })
const notice = ref('')
useSuccessToast(() => notice.value, { onDismiss: () => (notice.value = '') })
const unavailable = ref(false)

const skillModalOpen = ref(false)
const skillModalMode = ref<'create' | 'edit'>('create')
const activeSkillId = ref<string | null>(null)

let controller: AbortController | undefined

const skillsCatalog = computed(() => settings.value?.skills ?? [])
const builtinSkillsCount = computed(() => skillsCatalog.value.filter(s => s.is_builtin).length)
const customSkillsCount = computed(() => skillsCatalog.value.filter(s => !s.is_builtin).length)

const filteredSkills = computed(() => {
  const q = search.value.trim().toLowerCase()
  return skillsCatalog.value.filter(s => {
    const matchesFilter =
      skillFilter.value === 'all' || (skillFilter.value === 'builtin') === s.is_builtin
    if (!matchesFilter) return false
    if (!q) return true
    return `${s.id} ${s.name} ${s.description}`.toLowerCase().includes(q)
  })
})

function openCreateSkillModal() {
  skillModalMode.value = 'create'
  activeSkillId.value = null
  skillModalOpen.value = true
}

function openSkillDetail(id: string) {
  void router.push({ name: 'jobs-settings-skill', params: { skillId: id } })
}

function isManageableSkillId(id: string) {
  return /^[a-z0-9][a-z0-9_-]{0,63}$/.test(id)
}

function isSafeSkippedSkillId(id: string) {
  return id.length > 0 && id !== '.' && id !== '..' &&
    !id.includes('/') && !id.includes('\\') && !id.includes('\0')
}

function openSkillEditor(id: string) {
  skillModalMode.value = 'edit'
  activeSkillId.value = id
  skillModalOpen.value = true
}

function openSkillFromRow(skill: JobSkillSummary) {
  if (skill.status === 'invalid') {
    if (isManageableSkillId(skill.id)) openSkillEditor(skill.id)
    return
  }
  openSkillDetail(skill.id)
}

async function onSkillSaved() {
  skillModalOpen.value = false
  await load()
  notice.value = t('jobUi.skillSaved')
}

async function onSkillDeleted() {
  skillModalOpen.value = false
  await load()
  notice.value = t('jobUi.skillDeleted')
}

async function deleteCustomSkillFromCard(skill: JobSkillSummary) {
  if (skill.is_builtin || saving.value) return
  if (typeof window !== 'undefined' && typeof window.confirm === 'function') {
    const confirmed = window.confirm(t('jobUi.skillDeleteConfirm', { id: skill.id }))
    if (!confirmed) return
  }
  saving.value = true
  error.value = ''
  notice.value = ''
  try {
    await agentJobsApi.deleteSkill(skill.id, skill.etag)
    await load()
    notice.value = t('jobUi.skillDeleted')
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

async function load() {
  controller?.abort()
  const pending = new AbortController()
  controller = pending
  loading.value = true
  error.value = ''
  notice.value = ''
  try {
    const config = await agentJobsApi.settings(pending.signal)
    if (pending.signal.aborted) return
    settings.value = config
    unavailable.value = false
  } catch (cause) {
    if (pending.signal.aborted) return
    unavailable.value = isApiError(cause) && cause.status === 404
    error.value = formatApiError(cause)
  } finally {
    if (controller === pending) loading.value = false
  }
}

onMounted(() => {
  void load()
})
onBeforeUnmount(() => controller?.abort())
</script>

<template>
  <section class="space-y-4">
    <DetailHeader
      :title="t('jobUi.skillsPageTitle')"
      :back-label="t('jobUi.allSettingsBack')"
      @back="router.push({ name: 'jobs-settings' })"
    >
      <template #icon>
        <div
          class="grid size-9 shrink-0 place-items-center rounded-xl bg-studio-cyan/12 text-studio-cyan"
        >
          <BookOpen class="size-4.5" aria-hidden="true" />
        </div>
      </template>
      <template #details>
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span class="font-semibold uppercase tracking-wider text-studio-violet">
            {{ t('jobUi.hubEyebrow') }}
          </span>
          <span aria-hidden="true">·</span>
          <span>{{ t('jobUi.skillsPageSubtitle') }}</span>
        </div>
      </template>
      <template #actions>
        <Button variant="outline" size="sm" :disabled="loading || saving" @click="load">
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" aria-hidden="true" />
          {{ t('jobUi.refresh') }}
        </Button>
        <Button
          size="sm"
          type="button"
          :disabled="loading || saving"
          data-skill-create-btn
          @click="openCreateSkillModal"
        >
          <Plus class="size-4" aria-hidden="true" />
          {{ t('jobUi.skillAdd') }}
        </Button>
      </template>
    </DetailHeader>

    <!-- Toolbar: Filter Tabs + Search -->
    <div class="flex flex-wrap items-center justify-between gap-2.5">
      <div
        class="inline-flex rounded-lg border border-border bg-surface/80 p-0.5 text-xs"
        role="tablist"
      >
        <button
          type="button"
          role="tab"
          :aria-selected="skillFilter === 'all'"
          data-skill-filter="all"
          data-filter="all"
          class="cursor-pointer rounded-md px-3 py-1.5 font-medium transition-colors"
          :class="
            skillFilter === 'all'
              ? 'bg-card text-studio-violet shadow-xs'
              : 'text-muted-foreground hover:text-foreground'
          "
          @click="skillFilter = 'all'"
        >
          {{ t('jobUi.skillFilterAll') }} ({{ skillsCatalog.length }})
        </button>
        <button
          type="button"
          role="tab"
          :aria-selected="skillFilter === 'builtin'"
          data-skill-filter="builtin"
          data-filter="builtin"
          class="cursor-pointer rounded-md px-3 py-1.5 font-medium transition-colors"
          :class="
            skillFilter === 'builtin'
              ? 'bg-card text-studio-violet shadow-xs'
              : 'text-muted-foreground hover:text-foreground'
          "
          @click="skillFilter = 'builtin'"
        >
          {{ t('jobUi.skillFilterBuiltin') }} ({{ builtinSkillsCount }})
        </button>
        <button
          type="button"
          role="tab"
          :aria-selected="skillFilter === 'custom'"
          data-skill-filter="custom"
          data-filter="custom"
          class="cursor-pointer rounded-md px-3 py-1.5 font-medium transition-colors"
          :class="
            skillFilter === 'custom'
              ? 'bg-card text-studio-violet shadow-xs'
              : 'text-muted-foreground hover:text-foreground'
          "
          @click="skillFilter = 'custom'"
        >
          {{ t('jobUi.skillFilterCustom') }} ({{ customSkillsCount }})
        </button>
      </div>

      <div class="relative min-w-[200px] flex-1 sm:max-w-xs">
        <Search
          class="pointer-events-none absolute left-3 top-2.5 size-4 text-muted-foreground"
          aria-hidden="true"
        />
        <input
          v-model="search"
          type="search"
          class="admin-input h-9 pl-9 text-xs"
          :placeholder="t('jobUi.skillsSearchPlaceholder')"
          :aria-label="t('jobUi.skillsSearchPlaceholder')"
          data-skill-search
        />
      </div>
    </div>

    <div
      v-if="loading && !settings"
      class="studio-panel p-6 text-center text-sm text-muted-foreground"
      role="status"
    >
      {{ t('common.loading') }}
    </div>

    <!-- Skills Table -->
    <section
      v-else-if="settings"
      id="job-skill-catalog"
      class="studio-panel overflow-hidden"
      data-skill-catalog
    >
      <div class="overflow-x-auto">
        <table class="w-full min-w-[700px] border-collapse text-left text-xs">
          <thead>
            <tr class="border-b border-border/70 bg-surface/70 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
              <th class="px-4 py-2.5">{{ t('jobUi.skillsColSkill') }}</th>
              <th class="px-4 py-2.5">{{ t('jobUi.skillsColType') }}</th>
              <th class="px-4 py-2.5">{{ t('jobUi.skillsColDesc') }}</th>
              <th class="px-4 py-2.5">{{ t('jobUi.skillsColUsed') }}</th>
              <th class="px-4 py-2.5 text-right" />
            </tr>
          </thead>
          <tbody class="divide-y divide-border/60">
            <template v-if="filteredSkills.length > 0">
              <tr
                v-for="skill in filteredSkills"
                :key="skill.id"
                class="cursor-pointer transition-colors hover:bg-accent/30"
                :data-skill="skill.id"
                :data-skill-card="skill.id"
                @click="openSkillFromRow(skill)"
              >
                <td class="px-4 py-3">
                  <div class="flex items-center gap-2.5">
                    <BookOpen class="size-4 shrink-0 text-studio-violet" aria-hidden="true" />
                    <div>
                      <strong class="font-semibold text-foreground">{{ skill.name }}</strong>
                      <div class="mt-0.5 font-mono text-[11px] text-muted-foreground">
                        {{ skill.id }}
                      </div>
                    </div>
                  </div>
                </td>

                <td class="px-4 py-3 whitespace-nowrap">
                  <div class="flex flex-wrap items-center gap-1">
                    <Badge v-if="skill.status === 'invalid'" variant="outline" class="border-danger/40 text-danger">
                      {{ t('jobUi.skillInvalidBadge') }}
                    </Badge>
                    <Badge v-else :variant="skill.is_builtin ? 'secondary' : 'default'">
                      {{
                        skill.is_builtin
                          ? t('jobUi.skillBuiltinBadge')
                          : t('jobUi.skillCustomBadge')
                      }}
                    </Badge>
                    <Badge
                      v-if="skill.is_builtin && skill.is_modified"
                      variant="outline"
                      class="border-warning/40 bg-warning/10 text-warning-foreground dark:text-warning"
                    >
                      {{ t('jobUi.customizedBadge') }}
                    </Badge>
                    <Badge
                      v-if="skill.is_builtin && skill.default_has_update"
                      variant="outline"
                      class="border-studio-cyan/40 bg-studio-cyan/10 text-studio-cyan"
                      :data-skill-default-update="skill.id"
                    >
                      {{ t('jobUi.skillDefaultUpdateBadge') }}
                    </Badge>
                  </div>
                </td>

                <td class="max-w-md px-4 py-3 text-muted-foreground">
                  <span v-if="skill.status === 'invalid'" class="text-danger" :data-skill-invalid-reason="skill.id">
                    {{ skill.invalid_reason || t('jobUi.skillInvalidFallback') }}
                  </span>
                  <span v-else>{{ skill.description }}</span>
                </td>

                <td class="px-4 py-3 whitespace-nowrap text-muted-foreground">
                  {{ skill.used_by_agents.length }} Agents
                </td>

                <td class="px-4 py-3 text-right whitespace-nowrap">
                  <div class="inline-flex items-center gap-1.5">
                    <Button
                      v-if="!skill.is_builtin && (isManageableSkillId(skill.id) || (skill.status === 'invalid' && isSafeSkippedSkillId(skill.id)))"
                      size="sm"
                      variant="outline"
                      type="button"
                      class="h-7 border-danger/40 px-2 text-xs text-danger hover:bg-danger/10"
                      :disabled="saving"
                      :data-skill-delete-card="skill.id"
                      @click.stop="deleteCustomSkillFromCard(skill)"
                    >
                      <Trash2 class="size-3.5" aria-hidden="true" />
                    </Button>
                    <Button
                      v-if="skill.status === 'invalid' && isManageableSkillId(skill.id)"
                      size="sm"
                      variant="outline"
                      type="button"
                      class="h-7 px-2.5 text-xs"
                      :data-skill-repair="skill.id"
                      @click.stop="openSkillEditor(skill.id)"
                    >
                      {{ t('jobUi.skillRepair') }}
                    </Button>
                    <Button
                      v-if="skill.status !== 'invalid' || isManageableSkillId(skill.id)"
                      size="sm"
                      variant="outline"
                      type="button"
                      class="h-7 px-2.5 text-xs"
                      :data-skill-edit="skill.id"
                      :data-skill-detail="skill.id"
                      @click.stop="openSkillFromRow(skill)"
                    >
                      {{ t('jobUi.details') }}
                      <ArrowRight class="size-3.5" aria-hidden="true" />
                    </Button>
                  </div>
                </td>
              </tr>
            </template>
            <tr v-else>
              <td colspan="5" class="px-4 py-6 text-center text-muted-foreground">
                {{ t('jobUi.skillsEmptySearch') }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>

    <JobSkillEditorModal
      :open="skillModalOpen"
      :mode="skillModalMode"
      :skill-id="activeSkillId"
      @close="skillModalOpen = false"
      @saved="(s: JobSkillDetail) => void onSkillSaved()"
      @deleted="() => void onSkillDeleted()"
    />
  </section>
</template>

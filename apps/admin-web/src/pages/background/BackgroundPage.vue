<script setup lang="ts">
import {
  Activity, ArrowRight, BookOpen, Bot, Cpu, ExternalLink, Pencil, Plus, RefreshCw,
  Save, Settings2, Sliders, Trash2, Wrench,
} from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { agentJobsApi } from '@/api/agent-jobs'
import type {
  BackgroundAgentConfig, BackgroundAgentConfigUpdate, BackgroundProviderOption,
  BackgroundSettings, BackgroundSettingsUpdate, JobSkillDetail, JobSkillSummary,
} from '@/api/types/agent-jobs'
import { formatApiError, isApiError } from '@/api/errors'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import {
  chatProviderOptions as filterChatProviderOptions,
  domainToolsForAgent,
  findProvider as findCatalogProvider,
  formatProviderOptionLabel,
  isAgentCustomizedFromDefault,
  isAgentDirty as checkAgentDirty,
  providerOptionsForAgent as filterProviderOptionsForAgent,
  toAgentUpdate,
} from './backgroundAgentUtils'
import JobSkillEditorModal from './JobSkillEditorModal.vue'

const router = useRouter()
const { t } = useI18n()
const settings = ref<BackgroundSettings | null>(null)
const draft = ref<BackgroundSettingsUpdate>({
  enabled: false,
  orchestrator_llm_provider: null,
  worker_llm_provider: null,
  agents: {},
})
const loading = ref(true)
const saving = ref(false)
const error = ref('')
const notice = ref('')
const unavailable = ref(false)

let controller: AbortController | undefined

const providerOptions = computed(() => settings.value?.llm_providers ?? [])
const chatProviderOptions = computed(() => filterChatProviderOptions(providerOptions.value))
function providerOptionsForAgent(agent: BackgroundAgentConfig | null): BackgroundProviderOption[] {
  return filterProviderOptionsForAgent(providerOptions.value, agent)
}
const agentsCatalog = computed(() => settings.value?.agents ?? [])
const skillsCatalog = computed(() => settings.value?.skills ?? [])
const dailyFlowAgents = computed(() => agentsCatalog.value.filter(a => a.flow_id === 'daily'))
const wikiFlowAgents = computed(() => agentsCatalog.value.filter(a => a.flow_id === 'wiki'))

const skillModalOpen = ref(false)
const skillModalMode = ref<'create' | 'edit'>('create')
const activeSkillId = ref<string | null>(null)

function openCreateSkillModal() {
  skillModalMode.value = 'create'
  activeSkillId.value = null
  skillModalOpen.value = true
}

function openEditSkillModal(id: string) {
  skillModalMode.value = 'edit'
  activeSkillId.value = id
  skillModalOpen.value = true
}

async function refreshCatalogAfterSkillChange(deletedId?: string) {
  const wasDirty = dirty.value
  try {
    const config = await agentJobsApi.settings()
    settings.value = config
    if (!wasDirty) {
      restore(config)
    } else if (deletedId && draft.value.agents) {
      for (const ag of Object.values(draft.value.agents)) {
        if (ag.allowed_skills) {
          ag.allowed_skills = ag.allowed_skills.filter(s => s !== deletedId)
        }
      }
    }
  } catch (cause) {
    error.value = formatApiError(cause)
  }
}

async function onSkillSaved(_saved: JobSkillDetail) {
  skillModalOpen.value = false
  await refreshCatalogAfterSkillChange()
  notice.value = t('jobUi.skillSaved')
}

async function onSkillDeleted(deletedId: string) {
  skillModalOpen.value = false
  await refreshCatalogAfterSkillChange(deletedId)
  notice.value = t('jobUi.skillDeleted')
}

async function deleteCustomSkillFromCard(skill: JobSkillSummary) {
  if (skill.is_builtin) return
  if (typeof window !== 'undefined' && typeof window.confirm === 'function') {
    const confirmed = window.confirm(t('jobUi.skillDeleteConfirm', { id: skill.id }))
    if (!confirmed) return
  }
  error.value = ''
  notice.value = ''
  try {
    await agentJobsApi.deleteSkill(skill.id, skill.etag)
    await onSkillDeleted(skill.id)
  } catch (cause) {
    error.value = formatApiError(cause)
  }
}

function findProvider(refId: string | null | undefined): BackgroundProviderOption | undefined {
  return findCatalogProvider(providerOptions.value, refId)
}

function isAgentDirty(agent: BackgroundAgentConfig): boolean {
  return checkAgentDirty(agent, draft.value.agents?.[agent.id])
}

const dirty = computed(() => {
  if (!settings.value) return false
  if (
    settings.value.enabled !== draft.value.enabled ||
    settings.value.orchestrator_llm_provider !== draft.value.orchestrator_llm_provider ||
    settings.value.worker_llm_provider !== draft.value.worker_llm_provider
  ) {
    return true
  }
  return agentsCatalog.value.some(isAgentDirty)
})

const readyToSave = computed(() => Boolean(
  settings.value && dirty.value && (!draft.value.enabled ||
  (draft.value.orchestrator_llm_provider && draft.value.worker_llm_provider)),
))

function restore(value: BackgroundSettings) {
  const agentDrafts: Record<string, BackgroundAgentConfigUpdate> = {}
  for (const agent of value.agents ?? []) {
    agentDrafts[agent.id] = toAgentUpdate(agent)
  }
  draft.value = {
    enabled: value.enabled,
    orchestrator_llm_provider: value.orchestrator_llm_provider,
    worker_llm_provider: value.worker_llm_provider,
    agents: agentDrafts,
  }
}

function draftAgent(agent: BackgroundAgentConfig): BackgroundAgentConfigUpdate {
  if (!draft.value.agents) draft.value.agents = {}
  if (!draft.value.agents[agent.id]) {
    draft.value.agents[agent.id] = toAgentUpdate(agent)
  }
  return draft.value.agents[agent.id]!
}

function effectiveProviderForAgent(agent: BackgroundAgentConfig): BackgroundProviderOption | undefined {
  const d = draftAgent(agent)
  const explicitRef = d.provider_ref
  const fallbackRef = agent.mode === 'primary'
    ? draft.value.orchestrator_llm_provider
    : draft.value.worker_llm_provider
  return findProvider(explicitRef || fallbackRef || agent.effective_provider_ref)
}

function activeSubagentsForRoot(root: BackgroundAgentConfig): string[] {
  const flowSubagents = agentsCatalog.value.filter(
    a => a.flow_id === root.flow_id && a.mode !== 'primary',
  )
  return flowSubagents
    .filter(sub => draftAgent(sub).enabled)
    .map(sub => sub.id)
}

function openConfigurePage(agent: BackgroundAgentConfig) {
  void router.push({ name: 'jobs-settings-agent', params: { agentId: agent.id } })
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
    restore(config)
    unavailable.value = false
  } catch (cause) {
    if (pending.signal.aborted) return
    unavailable.value = isApiError(cause) && cause.status === 404
    error.value = formatApiError(cause)
  } finally {
    if (controller === pending) loading.value = false
  }
}

async function save() {
  if (!settings.value || !readyToSave.value || saving.value) return
  saving.value = true
  notice.value = ''
  error.value = ''
  try {
    const payload: BackgroundSettingsUpdate = {
      enabled: draft.value.enabled,
      orchestrator_llm_provider: draft.value.orchestrator_llm_provider,
      worker_llm_provider: draft.value.worker_llm_provider,
      ...(draft.value.agents && Object.keys(draft.value.agents).length > 0
        ? { agents: draft.value.agents }
        : {}),
    }
    const result = await agentJobsApi.updateSettings(payload, settings.value.revision)
    settings.value = {
      ...result,
      llm_providers: result.llm_providers ?? settings.value.llm_providers,
      agents: result.agents ?? settings.value.agents,
    }
    restore(settings.value)
    notice.value = t('jobUi.saved')
  } catch (cause) {
    error.value = formatApiError(cause)
    if (isApiError(cause) && cause.status === 409) {
      settings.value = null
      await load()
    }
  } finally {
    saving.value = false
  }
}

onMounted(() => { void load() })
onBeforeUnmount(() => controller?.abort())
</script>

<template>
  <section class="space-y-5">
    <DetailHeader :title="t('jobUi.background')" :back-label="t('jobUi.jobs')" @back="router.push({ name: 'jobs' })">
      <template #icon>
        <div class="grid size-10 shrink-0 place-items-center rounded-xl bg-studio-violet/10 text-studio-violet">
          <Settings2 class="size-5" />
        </div>
      </template>
      <template #details>
        <span class="line-clamp-2">{{ t('jobUi.backgroundDescription') }}</span>
      </template>
      <template #actions>
        <Button variant="outline" :disabled="loading || saving" @click="router.push({ name: 'providers', query: { type: 'llm' } })">
          <ExternalLink class="size-4" />{{ t('jobUi.manageProviders') }}
        </Button>
        <Button variant="outline" :disabled="loading || saving" @click="load">
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" />{{ t('jobUi.refresh') }}
        </Button>
        <Button :disabled="!readyToSave || saving" @click="save">
          <Save class="size-4" />{{ t('jobUi.save') }}
        </Button>
      </template>
    </DetailHeader>

    <div v-if="error" role="alert" class="rounded-lg border border-danger/30 bg-danger/5 p-4 text-sm text-danger">
      <p class="font-medium">{{ unavailable ? t('jobUi.backendPending') : t('jobUi.loadFailed') }}</p>
      <p class="mt-1 break-words">{{ error }}</p>
    </div>
    <p v-if="notice" role="status" class="rounded-lg border border-success/30 bg-success/5 p-3 text-sm text-success-foreground">{{ notice }}</p>

    <div class="rounded-xl border border-studio-violet/20 bg-studio-violet/5 p-4 text-sm text-foreground">
      <strong>{{ t('jobUi.globalScope') }}</strong> {{ t('jobUi.allAgents') }}
    </div>
    <div v-if="loading" class="studio-panel p-8 text-center text-sm text-muted-foreground" role="status">{{ t('common.loading') }}</div>

    <template v-if="settings && !loading">
      <section class="studio-panel p-5">
        <div class="flex flex-wrap items-center justify-between gap-5">
          <div class="flex items-start gap-3">
            <div class="grid size-10 shrink-0 place-items-center rounded-xl bg-studio-violet/10 text-studio-violet"><Activity class="size-5" /></div>
            <div>
              <h2 class="font-semibold">{{ t('jobUi.globalRuntime') }}</h2>
              <p class="mt-1 text-sm text-muted-foreground">{{ t('jobUi.globalHint') }}</p>
            </div>
          </div>
          <label class="flex cursor-pointer items-center gap-3 text-sm font-medium">
            <span>{{ draft.enabled ? t('jobUi.on') : t('jobUi.off') }}</span>
            <span class="relative inline-flex shrink-0 items-center">
              <input v-model="draft.enabled" type="checkbox" role="switch" :aria-label="t('jobUi.globalRuntime')"
                class="peer sr-only" :disabled="saving" />
              <span aria-hidden="true" class="h-6 w-11 rounded-full bg-muted-foreground/40 transition-colors peer-checked:bg-primary peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-ring peer-disabled:opacity-40"></span>
              <span aria-hidden="true" class="pointer-events-none absolute left-[3px] size-[18px] rounded-full bg-white shadow transition-transform peer-checked:translate-x-[18px]"></span>
            </span>
          </label>
        </div>
      </section>

      <section class="studio-panel space-y-5 p-5">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <h2 class="flex items-center gap-2 font-semibold"><Settings2 class="size-5 text-studio-violet" />{{ t('jobUi.providerConfiguration') }}</h2>
          <span class="text-xs text-muted-foreground">{{ t('jobUi.deploymentWide') }}</span>
        </div>
        <div class="grid gap-4 md:grid-cols-2">
          <div class="min-w-0 rounded-xl border bg-background p-4">
            <div class="mb-4 flex items-center gap-3">
              <span class="grid size-8 shrink-0 place-items-center rounded-lg bg-studio-violet/10 font-semibold text-studio-violet">1</span>
              <div><h3 class="flex items-center gap-2 font-semibold"><Bot class="size-4" />{{ t('jobUi.orchestrator') }}</h3><p class="mt-1 text-xs text-muted-foreground">{{ t('jobUi.orchestratorHint') }}</p></div>
            </div>
            <label class="block space-y-2">
              <span class="text-xs font-medium text-muted-foreground">{{ t('jobUi.providers') }}</span>
              <select v-model="draft.orchestrator_llm_provider" class="admin-input" :disabled="saving">
                <option :value="null">{{ t('jobUi.chooseProvider') }}</option>
                <option v-for="option in chatProviderOptions" :key="option.ref" :value="option.ref">
                  {{ formatProviderOptionLabel(option) }}
                </option>
                <option v-if="draft.orchestrator_llm_provider && !chatProviderOptions.some(p => p.ref === draft.orchestrator_llm_provider)"
                  :value="draft.orchestrator_llm_provider">{{ draft.orchestrator_llm_provider }} ({{ t('jobUi.unavailable') }})</option>
              </select>
            </label>
            <div v-if="findProvider(draft.orchestrator_llm_provider)" class="mt-3 flex flex-wrap gap-1.5 text-xs">
              <Badge v-if="findProvider(draft.orchestrator_llm_provider)?.model" variant="secondary">
                model: {{ findProvider(draft.orchestrator_llm_provider)?.model }}
              </Badge>
              <Badge v-for="tag in findProvider(draft.orchestrator_llm_provider)?.tags ?? []" :key="tag" variant="outline">
                tag: {{ tag }}
              </Badge>
              <Badge v-if="findProvider(draft.orchestrator_llm_provider)?.max_tokens" variant="outline">
                max_tokens: {{ findProvider(draft.orchestrator_llm_provider)?.max_tokens }}
              </Badge>
              <Badge v-if="findProvider(draft.orchestrator_llm_provider)?.context_limit_tokens" variant="outline">
                ctx: {{ findProvider(draft.orchestrator_llm_provider)?.context_limit_tokens }}
              </Badge>
              <Badge v-if="findProvider(draft.orchestrator_llm_provider)?.reasoning_effort" variant="outline">
                reasoning: {{ findProvider(draft.orchestrator_llm_provider)?.reasoning_effort }}
              </Badge>
            </div>
            <p class="mt-3 text-xs text-muted-foreground">{{ t('jobUi.providerCatalogHint') }}</p>
          </div>

          <div class="min-w-0 rounded-xl border bg-background p-4">
            <div class="mb-4 flex items-center gap-3">
              <span class="grid size-8 shrink-0 place-items-center rounded-lg bg-studio-violet/10 font-semibold text-studio-violet">2</span>
              <div><h3 class="flex items-center gap-2 font-semibold"><Cpu class="size-4" />{{ t('jobUi.worker') }}</h3><p class="mt-1 text-xs text-muted-foreground">{{ t('jobUi.workerHint') }}</p></div>
            </div>
            <label class="block space-y-2">
              <span class="text-xs font-medium text-muted-foreground">{{ t('jobUi.providers') }}</span>
              <select v-model="draft.worker_llm_provider" class="admin-input" :disabled="saving">
                <option :value="null">{{ t('jobUi.chooseProvider') }}</option>
                <option v-for="option in chatProviderOptions" :key="option.ref" :value="option.ref">
                  {{ formatProviderOptionLabel(option) }}
                </option>
                <option v-if="draft.worker_llm_provider && !chatProviderOptions.some(p => p.ref === draft.worker_llm_provider)"
                  :value="draft.worker_llm_provider">{{ draft.worker_llm_provider }} ({{ t('jobUi.unavailable') }})</option>
              </select>
            </label>
            <div v-if="findProvider(draft.worker_llm_provider)" class="mt-3 flex flex-wrap gap-1.5 text-xs">
              <Badge v-if="findProvider(draft.worker_llm_provider)?.model" variant="secondary">
                model: {{ findProvider(draft.worker_llm_provider)?.model }}
              </Badge>
              <Badge v-for="tag in findProvider(draft.worker_llm_provider)?.tags ?? []" :key="tag" variant="outline">
                tag: {{ tag }}
              </Badge>
              <Badge v-if="findProvider(draft.worker_llm_provider)?.max_tokens" variant="outline">
                max_tokens: {{ findProvider(draft.worker_llm_provider)?.max_tokens }}
              </Badge>
              <Badge v-if="findProvider(draft.worker_llm_provider)?.context_limit_tokens" variant="outline">
                ctx: {{ findProvider(draft.worker_llm_provider)?.context_limit_tokens }}
              </Badge>
              <Badge v-if="findProvider(draft.worker_llm_provider)?.reasoning_effort" variant="outline">
                reasoning: {{ findProvider(draft.worker_llm_provider)?.reasoning_effort }}
              </Badge>
            </div>
            <p class="mt-3 text-xs text-muted-foreground">{{ t('jobUi.providerCatalogHint') }}</p>
          </div>
        </div>
        <div class="flex flex-wrap items-center justify-between gap-3 border-t pt-4">
          <div>
            <p class="text-xs text-muted-foreground">{{ t('jobUi.sharedProviderHint') }}</p>
            <p v-if="draft.enabled && (!draft.orchestrator_llm_provider || !draft.worker_llm_provider)" class="mt-1 text-xs text-warning">
              {{ t('jobUi.providersRequired') }}
            </p>
          </div>
          <Button :disabled="!readyToSave || saving" @click="save"><Save class="size-4" />{{ t('jobUi.save') }}</Button>
        </div>
      </section>

      <!-- Per-Flow Root Agent & Sub-Agent Configuration -->
      <section v-for="group in [
        { key: 'daily', title: t('jobUi.flowDailyTitle'), hint: t('jobUi.flowDailyHint'), items: dailyFlowAgents },
        { key: 'wiki', title: t('jobUi.flowWikiTitle'), hint: t('jobUi.flowWikiHint'), items: wikiFlowAgents },
      ]" :key="group.key" v-show="group.items.length > 0" class="studio-panel space-y-4 p-5">
        <div class="flex flex-wrap items-start justify-between gap-3">
          <div>
            <h2 class="flex items-center gap-2 font-semibold">
              <Sliders class="size-5 text-studio-violet" />{{ group.title }}
            </h2>
            <p class="mt-1 text-xs text-muted-foreground">{{ group.hint }}</p>
          </div>
          <Badge variant="outline">{{ group.items.length }} agents</Badge>
        </div>

        <div class="space-y-4">
          <div
            v-for="agent in group.items.filter(a => a.mode === 'primary')"
            :key="agent.id"
            class="flex flex-col justify-between rounded-xl border border-studio-violet/30 bg-studio-violet/5 p-4"
          >
            <div class="space-y-3">
              <div class="flex flex-wrap items-center justify-between gap-2">
                <div class="flex items-center gap-2">
                  <button
                    type="button"
                    class="cursor-pointer rounded bg-muted px-2 py-0.5 font-mono text-sm font-semibold hover:text-studio-violet"
                    @click="openConfigurePage(agent)"
                  >
                    {{ agent.id }}
                  </button>
                  <Badge variant="default">{{ t('jobUi.primaryAgent') }}</Badge>
                  <Badge v-if="isAgentCustomizedFromDefault(agent, draftAgent(agent))" variant="outline">
                    {{ t('jobUi.customizedBadge') }}
                  </Badge>
                </div>
                <Button size="sm" variant="outline" :disabled="saving" @click="openConfigurePage(agent)">
                  <Wrench class="size-3.5" />{{ t('jobUi.configureAgent') }}<ArrowRight class="size-3.5" />
                </Button>
              </div>
              <p class="text-xs text-muted-foreground">
                {{ draftAgent(agent).description || agent.description }}
              </p>
              <label class="block space-y-1">
                <span class="text-xs font-medium text-muted-foreground">{{ t('jobUi.providers') }}</span>
                <select
                  v-model="draftAgent(agent).provider_ref"
                  class="admin-input h-9 text-xs"
                  :disabled="saving"
                >
                  <option :value="null">
                    {{ t('jobUi.inheritProvider') }} ({{ draft.orchestrator_llm_provider }})
                  </option>
                  <option v-for="option in providerOptionsForAgent(agent)" :key="option.ref" :value="option.ref">
                    {{ formatProviderOptionLabel(option) }}
                  </option>
                </select>
              </label>
            </div>
            <div class="mt-3 flex flex-wrap items-center gap-1.5 border-t pt-3 text-xs">
              <Badge variant="secondary">
                model: {{ effectiveProviderForAgent(agent)?.model ?? agent.effective_model }}
              </Badge>
              <Badge variant="outline">
                {{ t('jobUi.stepsLimit') }}: {{ draftAgent(agent).steps ?? agent.steps }}
              </Badge>
              <Badge v-if="effectiveProviderForAgent(agent)?.max_tokens" variant="outline">
                max_tokens: {{ effectiveProviderForAgent(agent)?.max_tokens }}
              </Badge>
              <Badge variant="outline">
                {{ t('jobUi.subagentsAllowed') }}: {{ activeSubagentsForRoot(agent).length }}
              </Badge>
            </div>
          </div>

          <div class="grid gap-4 lg:grid-cols-2">
            <div
              v-for="agent in group.items.filter(a => a.mode !== 'primary')"
              :key="agent.id"
              class="flex flex-col justify-between rounded-xl border bg-background p-4 transition-opacity"
              :class="{ 'opacity-60': !draftAgent(agent).enabled }"
            >
              <div class="space-y-3">
                <div class="flex flex-wrap items-center justify-between gap-2">
                  <div class="flex items-center gap-2">
                    <button
                      type="button"
                      class="cursor-pointer rounded bg-muted px-2 py-0.5 font-mono text-sm font-semibold hover:text-studio-violet"
                      @click="openConfigurePage(agent)"
                    >
                      {{ agent.id }}
                    </button>
                    <Badge variant="secondary">{{ t('jobUi.subagentRole') }}</Badge>
                    <Badge v-if="isAgentCustomizedFromDefault(agent, draftAgent(agent))" variant="outline">
                      {{ t('jobUi.customizedBadge') }}
                    </Badge>
                  </div>
                  <div class="flex items-center gap-2">
                    <label class="flex cursor-pointer items-center gap-2 text-xs font-medium">
                      <span>{{ draftAgent(agent).enabled ? t('jobUi.on') : t('jobUi.off') }}</span>
                      <input
                        v-model="draftAgent(agent).enabled"
                        type="checkbox"
                        class="size-4 accent-primary"
                        :disabled="saving"
                      />
                    </label>
                    <Button size="sm" variant="outline" :disabled="saving" @click="openConfigurePage(agent)">
                      <Wrench class="size-3.5" />{{ t('jobUi.configureAgent') }}<ArrowRight class="size-3.5" />
                    </Button>
                  </div>
                </div>

                <p class="line-clamp-2 text-xs text-muted-foreground">
                  {{ draftAgent(agent).description || agent.description }}
                </p>

                <label class="block space-y-1">
                  <span class="text-xs font-medium text-muted-foreground">{{ t('jobUi.providers') }}</span>
                  <select
                    v-model="draftAgent(agent).provider_ref"
                    class="admin-input h-9 text-xs"
                    :disabled="saving || !draftAgent(agent).enabled"
                  >
                    <option :value="null">
                      {{ t('jobUi.inheritProvider') }} ({{ draft.worker_llm_provider }})
                    </option>
                    <option v-for="option in providerOptionsForAgent(agent)" :key="option.ref" :value="option.ref">
                      {{ formatProviderOptionLabel(option) }}
                    </option>
                  </select>
                </label>
              </div>

              <div class="mt-3 flex flex-wrap items-center gap-1.5 border-t pt-3 text-xs">
                <Badge variant="secondary">
                  model: {{ effectiveProviderForAgent(agent)?.model ?? agent.effective_model }}
                </Badge>
                <Badge variant="outline">
                  {{ t('jobUi.stepsLimit') }}: {{ draftAgent(agent).steps ?? agent.steps }}
                </Badge>
                <Badge v-if="effectiveProviderForAgent(agent)?.max_tokens" variant="outline">
                  max_tokens: {{ effectiveProviderForAgent(agent)?.max_tokens }}
                </Badge>
                <Badge variant="outline">
                  {{ t('jobUi.toolsAllowed') }}: {{ (draftAgent(agent).allowed_tools ?? agent.allowed_tools).filter(t => t !== 'skill').length }}/{{ domainToolsForAgent(agent).length }}
                </Badge>
                <Badge variant="outline">
                  {{ t('jobUi.skillsAllowed') }}: {{ (draftAgent(agent).allowed_skills ?? agent.allowed_skills).length }}/{{ agent.available_skills.length }}
                </Badge>
              </div>
            </div>
          </div>
        </div>
      </section>

      <!-- Job Skill Catalog (skills/) -->
      <section class="studio-panel space-y-4 p-5" data-skill-catalog>
        <div class="flex flex-wrap items-start justify-between gap-3">
          <div>
            <h2 class="flex items-center gap-2 font-semibold">
              <BookOpen class="size-5 text-studio-violet" />{{ t('jobUi.skillCatalogTitle') }}
            </h2>
            <p class="mt-1 text-xs text-muted-foreground">{{ t('jobUi.skillCatalogHint') }}</p>
          </div>
          <div class="flex items-center gap-2">
            <Badge variant="outline">{{ skillsCatalog.length }} skills</Badge>
            <Button
              size="sm"
              type="button"
              :disabled="saving"
              data-skill-create-btn
              @click="openCreateSkillModal"
            >
              <Plus class="size-4" />{{ t('jobUi.skillAdd') }}
            </Button>
          </div>
        </div>

        <div class="grid gap-3 md:grid-cols-2">
          <div
            v-for="skill in skillsCatalog"
            :key="skill.id"
            class="flex flex-col justify-between rounded-xl border bg-background p-4"
            :data-skill-card="skill.id"
          >
            <div class="space-y-2.5">
              <div class="flex flex-wrap items-center justify-between gap-2">
                <div class="flex flex-wrap items-center gap-1.5">
                  <button
                    type="button"
                    class="cursor-pointer rounded bg-muted px-2 py-0.5 font-mono text-xs font-semibold hover:text-studio-violet"
                    @click="openEditSkillModal(skill.id)"
                  >
                    {{ skill.id }}
                  </button>
                  <Badge :variant="skill.is_builtin ? 'secondary' : 'outline'">
                    {{ skill.is_builtin ? t('jobUi.skillBuiltinBadge') : t('jobUi.skillCustomBadge') }}
                  </Badge>
                  <Badge v-if="skill.is_builtin && skill.is_modified" variant="outline">
                    {{ t('jobUi.customizedBadge') }}
                  </Badge>
                </div>

                <div class="flex items-center gap-1.5">
                  <Button
                    size="sm"
                    variant="outline"
                    type="button"
                    :data-skill-edit="skill.id"
                    @click="openEditSkillModal(skill.id)"
                  >
                    <Pencil class="size-3.5" />{{ t('jobUi.skillViewEdit') }}
                  </Button>
                  <Button
                    v-if="!skill.is_builtin"
                    size="sm"
                    variant="outline"
                    type="button"
                    class="border-danger/40 text-danger hover:bg-danger/10"
                    :data-skill-delete-card="skill.id"
                    @click="deleteCustomSkillFromCard(skill)"
                  >
                    <Trash2 class="size-3.5" />{{ t('common.delete') }}
                  </Button>
                </div>
              </div>

              <p class="text-xs font-medium text-foreground">{{ skill.name }}</p>
              <p class="line-clamp-2 text-xs text-muted-foreground">{{ skill.description }}</p>
            </div>

            <div class="mt-3 flex flex-wrap items-center gap-1.5 border-t pt-3 text-xs">
              <span class="text-muted-foreground">{{ t('jobUi.skillUsedBy') }}:</span>
              <template v-if="skill.used_by_agents.length > 0">
                <Badge v-for="ag in skill.used_by_agents" :key="ag" variant="secondary">
                  {{ ag }}
                </Badge>
              </template>
              <span v-else class="text-muted-foreground">{{ t('jobUi.skillUnused') }}</span>
            </div>
          </div>
        </div>
      </section>
    </template>

    <JobSkillEditorModal
      :open="skillModalOpen"
      :mode="skillModalMode"
      :skill-id="activeSkillId"
      @close="skillModalOpen = false"
      @saved="onSkillSaved"
      @deleted="onSkillDeleted"
    />
  </section>
</template>

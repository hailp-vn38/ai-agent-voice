<script setup lang="ts">
import {
  Activity, Bot, Cpu, ExternalLink, RefreshCw, RotateCcw,
  Save, Settings2, Sliders, Wrench,
} from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { agentJobsApi } from '@/api/agent-jobs'
import type {
  BackgroundAgentConfig, BackgroundAgentConfigUpdate, BackgroundProviderOption,
  BackgroundSettings, BackgroundSettingsUpdate,
} from '@/api/types/agent-jobs'
import { formatApiError, isApiError } from '@/api/errors'
import BaseModal from '@/components/admin/BaseModal.vue'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import NotificationSettingsCard from '@/components/notifications/NotificationSettingsCard.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

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

const editingAgentId = ref<string | null>(null)
const modalOpen = ref(false)
const modalForm = ref<{
  enabled: boolean
  provider_ref: string | null
  steps: number
  allowed_tools: string[]
  allowed_skills: string[]
  description: string
  system_prompt: string
}>({
  enabled: true,
  provider_ref: null,
  steps: 12,
  allowed_tools: [],
  allowed_skills: [],
  description: '',
  system_prompt: '',
})

let controller: AbortController | undefined

const providerOptions = computed(() => settings.value?.llm_providers ?? [])
const agentsCatalog = computed(() => settings.value?.agents ?? [])
const dailyFlowAgents = computed(() => agentsCatalog.value.filter(a => a.flow_id === 'daily'))
const wikiFlowAgents = computed(() => agentsCatalog.value.filter(a => a.flow_id === 'wiki'))
const editingAgentSpec = computed(() =>
  agentsCatalog.value.find(a => a.id === editingAgentId.value) ?? null,
)
const modalSelectedProvider = computed<BackgroundProviderOption | undefined>(() => {
  const spec = editingAgentSpec.value
  if (!spec) return undefined
  const fallbackRef = spec.mode === 'primary'
    ? draft.value.orchestrator_llm_provider
    : draft.value.worker_llm_provider
  return findProvider(modalForm.value.provider_ref || fallbackRef || spec.effective_provider_ref)
})

function findProvider(refId: string | null | undefined): BackgroundProviderOption | undefined {
  if (!refId) return undefined
  return providerOptions.value.find(p => p.ref === refId)
}

function normalizeTools(tools: string[], skills: string[], isPrimary: boolean): string[] {
  if (isPrimary) return ['subagent']
  const set = new Set(tools.filter(t => t !== 'skill'))
  const out = Array.from(set)
  if (skills.length > 0) out.unshift('skill')
  return out
}

function toAgentUpdate(agent: BackgroundAgentConfig): BackgroundAgentConfigUpdate {
  return {
    enabled: agent.enabled,
    provider_ref: agent.provider_ref ?? null,
    steps: agent.steps,
    allowed_tools: [...agent.allowed_tools],
    allowed_skills: [...agent.allowed_skills],
    description: agent.description,
    system_prompt: agent.system_prompt,
  }
}

function sameStringArray(a: string[] | undefined, b: string[] | undefined): boolean {
  const left = [...(a ?? [])].sort()
  const right = [...(b ?? [])].sort()
  return left.length === right.length && left.every((val, idx) => val === right[idx])
}

function isAgentDirty(agent: BackgroundAgentConfig): boolean {
  const d = draft.value.agents?.[agent.id]
  if (!d) return false
  return (
    d.enabled !== agent.enabled ||
    (d.provider_ref ?? null) !== (agent.provider_ref ?? null) ||
    (d.steps ?? agent.steps) !== agent.steps ||
    !sameStringArray(d.allowed_tools, agent.allowed_tools) ||
    !sameStringArray(d.allowed_skills, agent.allowed_skills) ||
    (d.description ?? agent.description).trim() !== agent.description.trim() ||
    (d.system_prompt ?? agent.system_prompt).trim() !== agent.system_prompt.trim()
  )
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

function domainToolsForAgent(agent: BackgroundAgentConfig): string[] {
  return agent.available_tools.filter(tool => tool !== 'skill')
}

function activeSubagentsForRoot(root: BackgroundAgentConfig): string[] {
  const flowSubagents = agentsCatalog.value.filter(
    a => a.flow_id === root.flow_id && a.mode !== 'primary',
  )
  return flowSubagents
    .filter(sub => draftAgent(sub).enabled)
    .map(sub => sub.id)
}

function openConfigureModal(agent: BackgroundAgentConfig) {
  const d = draftAgent(agent)
  editingAgentId.value = agent.id
  modalForm.value = {
    enabled: d.enabled,
    provider_ref: d.provider_ref ?? null,
    steps: d.steps ?? agent.steps,
    allowed_tools: [...(d.allowed_tools ?? agent.allowed_tools)].filter(t => t !== 'skill'),
    allowed_skills: [...(d.allowed_skills ?? agent.allowed_skills)],
    description: d.description ?? agent.description,
    system_prompt: d.system_prompt ?? agent.system_prompt,
  }
  modalOpen.value = true
}

function toggleModalTool(tool: string, checked: boolean) {
  const set = new Set(modalForm.value.allowed_tools)
  if (checked) set.add(tool)
  else set.delete(tool)
  modalForm.value.allowed_tools = Array.from(set)
}

function toggleModalSkill(skill: string, checked: boolean) {
  const set = new Set(modalForm.value.allowed_skills)
  if (checked) set.add(skill)
  else set.delete(skill)
  modalForm.value.allowed_skills = Array.from(set)
}

function resetModalToDefault() {
  const spec = editingAgentSpec.value
  if (!spec) return
  modalForm.value = {
    enabled: true,
    provider_ref: null,
    steps: spec.default_steps,
    allowed_tools: (spec.default_allowed_tools ?? spec.available_tools).filter(t => t !== 'skill'),
    allowed_skills: [...(spec.default_allowed_skills ?? [])],
    description: spec.default_description,
    system_prompt: spec.default_system_prompt,
  }
}

function applyModalChanges() {
  const spec = editingAgentSpec.value
  if (!spec) return
  if (!draft.value.agents) draft.value.agents = {}
  const steps = Math.max(1, Math.min(100, Number(modalForm.value.steps) || spec.default_steps))
  const skills = [...modalForm.value.allowed_skills]
  const tools = normalizeTools(modalForm.value.allowed_tools, skills, spec.mode === 'primary')
  draft.value.agents[spec.id] = {
    enabled: spec.mode === 'primary' ? true : modalForm.value.enabled,
    provider_ref: modalForm.value.provider_ref || null,
    steps,
    allowed_tools: tools,
    allowed_skills: skills,
    description: modalForm.value.description.trim() || spec.default_description,
    system_prompt: modalForm.value.system_prompt.trim() || spec.default_system_prompt,
  }
  modalOpen.value = false
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
                <option v-for="option in providerOptions" :key="option.ref" :value="option.ref">
                  {{ option.name }}{{ option.model && option.model !== option.name ? ` (${option.model})` : '' }} · {{ option.source }}
                </option>
                <option v-if="draft.orchestrator_llm_provider && !providerOptions.some(p => p.ref === draft.orchestrator_llm_provider)"
                  :value="draft.orchestrator_llm_provider">{{ draft.orchestrator_llm_provider }} ({{ t('jobUi.unavailable') }})</option>
              </select>
            </label>
            <div v-if="findProvider(draft.orchestrator_llm_provider)" class="mt-3 flex flex-wrap gap-1.5 text-xs">
              <Badge v-if="findProvider(draft.orchestrator_llm_provider)?.model" variant="secondary">
                model: {{ findProvider(draft.orchestrator_llm_provider)?.model }}
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
                <option v-for="option in providerOptions" :key="option.ref" :value="option.ref">
                  {{ option.name }}{{ option.model && option.model !== option.name ? ` (${option.model})` : '' }} · {{ option.source }}
                </option>
                <option v-if="draft.worker_llm_provider && !providerOptions.some(p => p.ref === draft.worker_llm_provider)"
                  :value="draft.worker_llm_provider">{{ draft.worker_llm_provider }} ({{ t('jobUi.unavailable') }})</option>
              </select>
            </label>
            <div v-if="findProvider(draft.worker_llm_provider)" class="mt-3 flex flex-wrap gap-1.5 text-xs">
              <Badge v-if="findProvider(draft.worker_llm_provider)?.model" variant="secondary">
                model: {{ findProvider(draft.worker_llm_provider)?.model }}
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
                  <code class="rounded bg-muted px-2 py-0.5 text-sm font-semibold">{{ agent.id }}</code>
                  <Badge variant="default">{{ t('jobUi.primaryAgent') }}</Badge>
                </div>
                <Button size="sm" variant="outline" :disabled="saving" @click="openConfigureModal(agent)">
                  <Wrench class="size-3.5" />{{ t('jobUi.configureAgent') }}
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
                  <option v-for="option in providerOptions" :key="option.ref" :value="option.ref">
                    {{ option.name }}{{ option.model && option.model !== option.name ? ` (${option.model})` : '' }} · {{ option.source }}
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
                    <code class="rounded bg-muted px-2 py-0.5 text-sm font-semibold">{{ agent.id }}</code>
                    <Badge variant="secondary">{{ t('jobUi.subagentRole') }}</Badge>
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
                    <Button size="sm" variant="outline" :disabled="saving" @click="openConfigureModal(agent)">
                      <Wrench class="size-3.5" />{{ t('jobUi.configureAgent') }}
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
                    <option v-for="option in providerOptions" :key="option.ref" :value="option.ref">
                      {{ option.name }}{{ option.model && option.model !== option.name ? ` (${option.model})` : '' }} · {{ option.source }}
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
    </template>

    <NotificationSettingsCard v-if="!loading" />

    <!-- Configure Agent / Sub-Agent Modal -->
    <BaseModal
      v-model="modalOpen"
      :title="editingAgentSpec ? `${t('jobUi.agentModalTitle')}: ${editingAgentSpec.id}` : t('jobUi.agentModalTitle')"
      width-class="max-w-3xl"
    >
      <div v-if="editingAgentSpec" class="space-y-4">
        <div class="grid gap-4 sm:grid-cols-2">
          <label class="block space-y-1.5">
            <span class="text-sm font-medium">{{ t('jobUi.providers') }}</span>
            <select v-model="modalForm.provider_ref" class="admin-input">
              <option :value="null">
                {{ t('jobUi.inheritProvider') }} ({{ editingAgentSpec.mode === 'primary' ? draft.orchestrator_llm_provider : draft.worker_llm_provider }})
              </option>
              <option v-for="option in providerOptions" :key="option.ref" :value="option.ref">
                {{ option.name }}{{ option.model && option.model !== option.name ? ` (${option.model})` : '' }} · {{ option.source }}
              </option>
            </select>
          </label>

          <label class="block space-y-1.5">
            <span class="text-sm font-medium">{{ t('jobUi.stepsLimit') }} (1 - 100)</span>
            <input
              v-model.number="modalForm.steps"
              type="number"
              min="1"
              max="100"
              class="admin-input"
            />
          </label>
        </div>

        <div
          v-if="modalSelectedProvider"
          class="flex flex-wrap items-center justify-between gap-2 rounded-lg border bg-muted/40 p-3 text-xs"
        >
          <div class="flex flex-wrap items-center gap-1.5">
            <Badge v-if="modalSelectedProvider.model" variant="secondary">
              model: {{ modalSelectedProvider.model }}
            </Badge>
            <Badge v-if="modalSelectedProvider.base_url" variant="outline">
              {{ modalSelectedProvider.base_url }}
            </Badge>
            <Badge v-if="modalSelectedProvider.max_tokens" variant="outline">
              max_tokens: {{ modalSelectedProvider.max_tokens }}
            </Badge>
            <Badge v-if="modalSelectedProvider.context_limit_tokens" variant="outline">
              ctx: {{ modalSelectedProvider.context_limit_tokens }}
            </Badge>
            <Badge v-if="modalSelectedProvider.reasoning_effort" variant="outline">
              reasoning: {{ modalSelectedProvider.reasoning_effort }}
            </Badge>
          </div>
          <Button
            size="sm"
            variant="ghost"
            @click="router.push(
              modalSelectedProvider.source === 'database'
                ? `/providers/${modalSelectedProvider.ref}`
                : { name: 'providers', query: { type: 'llm' } }
            )"
          >
            <ExternalLink class="size-3.5" />{{ t('jobUi.manageProviders') }}
          </Button>
        </div>

        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('jobUi.agentDescriptionLabel') }}</span>
          <input v-model="modalForm.description" type="text" maxlength="1024" class="admin-input" />
        </label>

        <div v-if="editingAgentSpec.mode !== 'primary' && domainToolsForAgent(editingAgentSpec).length" class="space-y-2">
          <span class="block text-sm font-medium">{{ t('jobUi.toolsAllowed') }}</span>
          <div class="grid grid-cols-2 gap-2 rounded-lg border p-3 sm:grid-cols-3">
            <label
              v-for="tool in domainToolsForAgent(editingAgentSpec)"
              :key="tool"
              class="flex cursor-pointer items-center gap-2 text-xs"
            >
              <input
                type="checkbox"
                class="size-4 accent-primary"
                :checked="modalForm.allowed_tools.includes(tool)"
                @change="toggleModalTool(tool, ($event.target as HTMLInputElement).checked)"
              />
              <code class="truncate">{{ tool }}</code>
            </label>
          </div>
        </div>

        <div v-if="editingAgentSpec.mode !== 'primary' && editingAgentSpec.available_skills.length" class="space-y-2">
          <span class="block text-sm font-medium">{{ t('jobUi.skillsAllowed') }}</span>
          <div class="flex flex-wrap gap-3 rounded-lg border p-3">
            <label
              v-for="skill in editingAgentSpec.available_skills"
              :key="skill"
              class="flex cursor-pointer items-center gap-2 text-xs"
            >
              <input
                type="checkbox"
                class="size-4 accent-primary"
                :checked="modalForm.allowed_skills.includes(skill)"
                @change="toggleModalSkill(skill, ($event.target as HTMLInputElement).checked)"
              />
              <code>{{ skill }}</code>
            </label>
          </div>
        </div>

        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('jobUi.systemPromptLabel') }}</span>
          <textarea
            v-model="modalForm.system_prompt"
            rows="12"
            class="admin-input font-mono text-xs leading-relaxed"
          />
        </label>
      </div>

      <template #footer>
        <div class="flex flex-wrap items-center justify-between gap-2">
          <Button variant="outline" type="button" @click="resetModalToDefault">
            <RotateCcw class="size-4" />{{ t('jobUi.resetAgentDefault') }}
          </Button>
          <div class="flex gap-2">
            <Button variant="outline" type="button" @click="modalOpen = false">Hủy</Button>
            <Button type="button" @click="applyModalChanges">{{ t('jobUi.applyAgentChanges') }}</Button>
          </div>
        </div>
      </template>
    </BaseModal>
  </section>
</template>

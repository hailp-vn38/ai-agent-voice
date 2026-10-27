<script setup lang="ts">
import {
  Activity, ArrowLeft, ArrowRight, BookOpen, Bot, Columns2, Cpu, Eye,
  ExternalLink, FileText, Pencil, RefreshCw, RotateCcw, Save,
  Settings2, Sliders, Wrench, Workflow,
} from '@lucide/vue'
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { agentJobsApi } from '@/api/agent-jobs'
import { formatApiError, isApiError } from '@/api/errors'
import type {
  BackgroundAgentConfig,
  BackgroundAgentConfigUpdate,
  BackgroundProviderOption,
  BackgroundSettings,
  BackgroundSettingsUpdate,
} from '@/api/types/agent-jobs'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { renderMarkdown } from '@/lib/markdown'
import { redactEndpoint } from '@/lib/providerEndpoint'
import {
  allowedCapabilityTagsForAgent,
  defaultDomainToolsForAgent,
  domainToolsForAgent,
  findProvider,
  formatProviderOptionLabel,
  isAgentCustomizedFromDefault,
  isAgentDirty,
  normalizeToolProviders,
  normalizeTools,
  providerOptionsForAgent,
  providerOptionsForTag,
  sameStringArray,
  toolCapabilityEntries,
} from './backgroundAgentUtils'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()

const agentId = computed(() => String(route.params.agentId ?? ''))
const settings = ref<BackgroundSettings | null>(null)
const loading = ref(true)
const saving = ref(false)
const error = ref('')
const notice = ref('')
const unavailable = ref(false)
const promptMode = ref<'edit' | 'preview' | 'split'>('edit')

const form = ref<{
  enabled: boolean
  provider_ref: string | null
  tool_providers: Record<string, string>
  steps: number
  allowed_tools: string[]
  allowed_skills: string[]
  description: string
  system_prompt: string
}>({
  enabled: true,
  provider_ref: null,
  tool_providers: {},
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

const agentSpec = computed<BackgroundAgentConfig | null>(
  () => agentsCatalog.value.find(a => a.id === agentId.value) ?? null,
)
const isPrimary = computed(() => agentSpec.value?.mode === 'primary')
const domainTools = computed(() => (agentSpec.value ? domainToolsForAgent(agentSpec.value) : []))
const defaultDomainTools = computed(() =>
  agentSpec.value ? defaultDomainToolsForAgent(agentSpec.value) : [],
)
const capabilityEntries = computed(() => toolCapabilityEntries(agentSpec.value))
const allowedCapabilityTags = computed(() => allowedCapabilityTagsForAgent(agentSpec.value))
const agentProviderOptions = computed(() =>
  providerOptionsForAgent(providerOptions.value, agentSpec.value),
)

const fallbackProviderRef = computed(() => {
  if (!settings.value || !agentSpec.value) return null
  return isPrimary.value
    ? settings.value.orchestrator_llm_provider
    : settings.value.worker_llm_provider
})

const selectedProvider = computed<BackgroundProviderOption | undefined>(() => {
  const spec = agentSpec.value
  if (!spec) return undefined
  return findProvider(
    providerOptions.value,
    form.value.provider_ref || fallbackProviderRef.value || spec.effective_provider_ref,
  )
})

const flowSubagents = computed(() => {
  const spec = agentSpec.value
  if (!spec) return []
  return agentsCatalog.value.filter(a => a.flow_id === spec.flow_id && a.mode !== 'primary')
})

const activeSubagents = computed(() => flowSubagents.value.filter(a => a.enabled))

const currentAgentUpdate = computed<BackgroundAgentConfigUpdate | undefined>(() => {
  const spec = agentSpec.value
  if (!spec) return undefined
  const steps = Math.max(1, Math.min(100, Number(form.value.steps) || spec.default_steps))
  const skills = [...form.value.allowed_skills]
  const tools = normalizeTools(form.value.allowed_tools, skills, spec.mode === 'primary')
  const toolProviders = normalizeToolProviders(form.value.tool_providers)
  return {
    enabled: spec.mode === 'primary' ? true : form.value.enabled,
    provider_ref: form.value.provider_ref || null,
    ...(capabilityEntries.value.length > 0 ? { tool_providers: toolProviders } : {}),
    steps,
    allowed_tools: tools,
    allowed_skills: skills,
    description: form.value.description.trim() || spec.default_description,
    system_prompt: form.value.system_prompt.trim() || spec.default_system_prompt,
  }
})

const dirty = computed(() => {
  if (!agentSpec.value || !currentAgentUpdate.value) return false
  return isAgentDirty(agentSpec.value, currentAgentUpdate.value)
})

const isCustomized = computed(() => {
  if (!agentSpec.value) return false
  return isAgentCustomizedFromDefault(agentSpec.value, currentAgentUpdate.value)
})

const isDescriptionModified = computed(() => {
  if (!agentSpec.value) return false
  return form.value.description.trim() !== agentSpec.value.default_description.trim()
})

const isPromptModified = computed(() => {
  if (!agentSpec.value) return false
  return form.value.system_prompt.trim() !== agentSpec.value.default_system_prompt.trim()
})

const promptLineCount = computed(() => {
  const text = form.value.system_prompt
  return text ? text.split(/\r?\n/).length : 0
})

const renderedPromptHtml = computed(() => renderMarkdown(form.value.system_prompt || ''))

function populateForm(spec: BackgroundAgentConfig) {
  const initialToolProviders: Record<string, string> = {}
  for (const [tool] of toolCapabilityEntries(spec)) {
    initialToolProviders[tool] = spec.tool_providers?.[tool] ?? ''
  }
  form.value = {
    enabled: spec.mode === 'primary' ? true : spec.enabled,
    provider_ref: spec.provider_ref ?? null,
    tool_providers: initialToolProviders,
    steps: spec.steps,
    allowed_tools: [...spec.allowed_tools].filter(t => t !== 'skill'),
    allowed_skills: [...spec.allowed_skills],
    description: spec.description,
    system_prompt: spec.system_prompt,
  }
}

function toggleTool(tool: string, checked: boolean) {
  const set = new Set(form.value.allowed_tools)
  if (checked) set.add(tool)
  else set.delete(tool)
  form.value.allowed_tools = Array.from(set)
}

function selectAllTools() {
  form.value.allowed_tools = [...domainTools.value]
}

function resetDefaultTools() {
  form.value.allowed_tools = [...defaultDomainTools.value]
}

function toggleSkill(skill: string, checked: boolean) {
  const set = new Set(form.value.allowed_skills)
  if (checked) set.add(skill)
  else set.delete(skill)
  form.value.allowed_skills = Array.from(set)
}

function selectAllSkills() {
  if (!agentSpec.value) return
  form.value.allowed_skills = [...agentSpec.value.available_skills]
}

function resetDefaultSkills() {
  if (!agentSpec.value) return
  form.value.allowed_skills = [...(agentSpec.value.default_allowed_skills ?? [])]
}

function resetToDefault() {
  const spec = agentSpec.value
  if (!spec) return
  const emptyToolProviders: Record<string, string> = {}
  for (const [tool] of toolCapabilityEntries(spec)) {
    emptyToolProviders[tool] = ''
  }
  form.value = {
    enabled: true,
    provider_ref: null,
    tool_providers: emptyToolProviders,
    steps: spec.default_steps,
    allowed_tools: [...defaultDomainToolsForAgent(spec)],
    allowed_skills: [...(spec.default_allowed_skills ?? [])],
    description: spec.default_description,
    system_prompt: spec.default_system_prompt,
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
    const spec = (config.agents ?? []).find(a => a.id === agentId.value)
    if (spec) populateForm(spec)
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
  const spec = agentSpec.value
  const update = currentAgentUpdate.value
  if (!settings.value || !spec || !update || !dirty.value || saving.value) return
  saving.value = true
  notice.value = ''
  error.value = ''
  try {
    const payload: BackgroundSettingsUpdate = {
      enabled: settings.value.enabled,
      orchestrator_llm_provider: settings.value.orchestrator_llm_provider,
      worker_llm_provider: settings.value.worker_llm_provider,
      agents: {
        [spec.id]: update,
      },
    }
    const result = await agentJobsApi.updateSettings(payload, settings.value.revision)
    settings.value = {
      ...result,
      llm_providers: result.llm_providers ?? settings.value.llm_providers,
      agents: result.agents ?? settings.value.agents,
    }
    const updatedSpec = (settings.value.agents ?? []).find(a => a.id === spec.id)
    if (updatedSpec) populateForm(updatedSpec)
    notice.value = t('jobUi.saved')
  } catch (cause) {
    error.value = formatApiError(cause)
    if (isApiError(cause) && cause.status === 409) {
      await load()
    }
  } finally {
    saving.value = false
  }
}

function switchAgent(targetId: string) {
  if (targetId === agentId.value) return
  void router.push({ name: 'jobs-settings-agent', params: { agentId: targetId } })
}

watch(
  agentId,
  () => {
    notice.value = ''
    error.value = ''
    const existing = agentsCatalog.value.find(a => a.id === agentId.value)
    if (existing) {
      populateForm(existing)
    } else {
      void load()
    }
  },
  { immediate: true },
)

onBeforeUnmount(() => controller?.abort())
</script>

<template>
  <main class="mx-auto max-w-7xl space-y-5">
    <DetailHeader
      :title="agentSpec ? agentSpec.id : (agentId || t('jobUi.agentModalTitle'))"
      :back-label="t('jobUi.background')"
      @back="router.push({ name: 'jobs-settings' })"
    >
      <template #icon>
        <span class="flex size-11 shrink-0 items-center justify-center rounded-xl border border-studio-violet/20 bg-studio-violet/10 text-studio-violet">
          <Bot v-if="isPrimary" class="size-5" aria-hidden="true" />
          <Cpu v-else class="size-5" aria-hidden="true" />
        </span>
      </template>
      <template v-if="agentSpec" #details>
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <Badge :variant="isPrimary ? 'default' : 'secondary'">
            {{ isPrimary ? t('jobUi.primaryAgent') : t('jobUi.subagentRole') }}
          </Badge>
          <span>{{ agentSpec.flow_id === 'daily' ? t('jobUi.flowDailyShort') : t('jobUi.flowWikiShort') }}</span>
          <template v-if="agentSpec.parent_id">
            <span aria-hidden="true">·</span>
            <span>{{ t('jobUi.parentCoordinator') }}: <code class="font-mono font-semibold text-foreground">{{ agentSpec.parent_id }}</code></span>
          </template>
          <span aria-hidden="true">·</span>
          <Badge :variant="form.enabled ? 'success' : 'outline'">
            {{ form.enabled ? t('jobUi.on') : t('jobUi.off') }}
          </Badge>
          <Badge :variant="isCustomized ? 'secondary' : 'outline'">
            {{ isCustomized ? t('jobUi.customizedBadge') : t('jobUi.defaultBadge') }}
          </Badge>
          <Badge v-if="dirty" variant="outline" class="border-studio-violet/40 text-studio-violet">
            {{ t('jobUi.unsavedChanges') }}
          </Badge>
        </div>
      </template>
      <template #actions>
        <Button
          variant="outline"
          size="sm"
          :disabled="loading || saving"
          @click="router.push({ name: 'providers', query: { type: 'llm' } })"
        >
          <ExternalLink class="size-4" aria-hidden="true" />{{ t('jobUi.manageProviders') }}
        </Button>
        <Button variant="outline" size="sm" :disabled="loading || saving" @click="load">
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" aria-hidden="true" />{{ t('jobUi.refresh') }}
        </Button>
        <Button
          v-if="agentSpec"
          variant="outline"
          size="sm"
          :disabled="loading || saving || !isCustomized"
          data-reset-agent-default
          @click="resetToDefault"
        >
          <RotateCcw class="size-4" aria-hidden="true" />{{ t('jobUi.resetAgentDefault') }}
        </Button>
        <Button
          v-if="agentSpec"
          size="sm"
          :disabled="!dirty || saving"
          data-save-agent-config
          @click="save"
        >
          <Save class="size-4" aria-hidden="true" />{{ t('jobUi.save') }}
        </Button>
      </template>
    </DetailHeader>

    <div v-if="error" role="alert" class="rounded-lg border border-danger/30 bg-danger/5 p-4 text-sm text-danger">
      <p class="font-medium">{{ unavailable ? t('jobUi.backendPending') : t('jobUi.loadFailed') }}</p>
      <p class="mt-1 break-words">{{ error }}</p>
    </div>
    <p v-if="notice" role="status" class="rounded-lg border border-success/30 bg-success/5 p-3 text-sm text-success-foreground">
      {{ notice }}
    </p>

    <div v-if="loading && !settings" class="studio-panel p-10 text-center text-sm text-muted-foreground" role="status">
      {{ t('common.loading') }}
    </div>

    <section v-else-if="!loading && !agentSpec" class="studio-panel space-y-4 p-10 text-center">
      <h2 class="text-xl font-semibold">{{ t('jobUi.agentNotFound') }}</h2>
      <p class="text-sm text-muted-foreground">{{ t('jobUi.agentNotFoundHint') }}</p>
      <Button variant="outline" @click="router.push({ name: 'jobs-settings' })">
        <ArrowLeft class="size-4" aria-hidden="true" />{{ t('jobUi.backToJobSettings') }}
      </Button>
    </section>

    <template v-else-if="agentSpec">
      <!-- Quick Agent Switcher Bar -->
      <nav
        v-if="agentsCatalog.length > 1"
        class="studio-panel flex flex-col gap-3 p-3.5 sm:flex-row sm:items-center sm:justify-between"
        :aria-label="t('jobUi.quickSwitchAgent')"
      >
        <div class="flex flex-wrap items-center gap-2">
          <span class="px-1 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            Flow 1:
          </span>
          <button
            v-for="item in dailyFlowAgents"
            :key="item.id"
            type="button"
            class="inline-flex cursor-pointer items-center gap-1.5 rounded-lg border px-2.5 py-1 text-xs transition-colors"
            :class="
              item.id === agentSpec.id
                ? 'border-studio-violet/40 bg-studio-violet/15 font-semibold text-studio-violet'
                : 'border-border/70 bg-surface/60 text-foreground hover:bg-accent/50'
            "
            @click="switchAgent(item.id)"
          >
            <span
              class="size-1.5 rounded-full"
              :class="item.enabled ? 'bg-success' : 'bg-muted-foreground/50'"
              aria-hidden="true"
            />
            <span class="font-mono">{{ item.id }}</span>
          </button>
        </div>

        <div v-if="wikiFlowAgents.length > 0" class="flex flex-wrap items-center gap-2">
          <span class="px-1 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            Flow 2:
          </span>
          <button
            v-for="item in wikiFlowAgents"
            :key="item.id"
            type="button"
            class="inline-flex cursor-pointer items-center gap-1.5 rounded-lg border px-2.5 py-1 text-xs transition-colors"
            :class="
              item.id === agentSpec.id
                ? 'border-studio-violet/40 bg-studio-violet/15 font-semibold text-studio-violet'
                : 'border-border/70 bg-surface/60 text-foreground hover:bg-accent/50'
            "
            @click="switchAgent(item.id)"
          >
            <span
              class="size-1.5 rounded-full"
              :class="item.enabled ? 'bg-success' : 'bg-muted-foreground/50'"
              aria-hidden="true"
            />
            <span class="font-mono">{{ item.id }}</span>
          </button>
        </div>
      </nav>

      <!-- Top Summary Stat Cards (consistent with ProviderDetailPage) -->
      <div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-4" aria-label="Agent configuration summary">
        <section class="studio-panel flex flex-col justify-between px-4 py-3.5">
          <div class="flex items-start justify-between gap-2">
            <div>
              <p class="text-xs text-muted-foreground">{{ t('jobUi.agentStatusCard') }}</p>
              <div class="mt-2 flex items-center gap-2 font-semibold">
                <span
                  class="size-2 rounded-full"
                  :class="form.enabled ? 'bg-success' : 'bg-muted-foreground'"
                  aria-hidden="true"
                />
                <span>{{ form.enabled ? t('jobUi.on') : t('jobUi.off') }}</span>
              </div>
            </div>
            <label v-if="!isPrimary" class="relative inline-flex cursor-pointer items-center">
              <input
                v-model="form.enabled"
                type="checkbox"
                role="switch"
                :aria-label="t('jobUi.agentStatusCard')"
                class="peer sr-only"
                :disabled="saving"
              />
              <span
                aria-hidden="true"
                class="h-6 w-11 rounded-full bg-muted-foreground/40 transition-colors peer-checked:bg-primary peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-ring peer-disabled:opacity-40"
              />
              <span
                aria-hidden="true"
                class="pointer-events-none absolute left-[3px] size-[18px] rounded-full bg-white shadow transition-transform peer-checked:translate-x-[18px]"
              />
            </label>
          </div>
          <p class="mt-2 text-xs text-muted-foreground">
            {{ isPrimary ? t('jobUi.primaryAlwaysActive') : t('jobUi.subagentToggleHint') }}
          </p>
        </section>

        <section class="studio-panel flex flex-col justify-between px-4 py-3.5">
          <div>
            <p class="text-xs text-muted-foreground">{{ t('jobUi.effectiveModelCard') }}</p>
            <p class="mt-2 truncate font-mono text-base font-semibold">
              {{ selectedProvider?.model ?? agentSpec.effective_model }}
            </p>
          </div>
          <p class="mt-2 truncate text-xs text-muted-foreground">
            {{
              form.provider_ref
                ? `${t('jobUi.dedicatedProviderOverride')} (${form.provider_ref})`
                : `${t('jobUi.inheritProvider')} (${fallbackProviderRef ?? agentSpec.effective_provider_ref})`
            }}
          </p>
        </section>

        <section class="studio-panel flex flex-col justify-between px-4 py-3.5">
          <div>
            <p class="text-xs text-muted-foreground">{{ t('jobUi.executionBudgetCard') }}</p>
            <div class="mt-2 flex items-baseline gap-2">
              <span class="font-mono text-lg font-semibold tabular-nums">{{ form.steps }}</span>
              <span class="text-xs text-muted-foreground">
                / 100 ({{ t('jobUi.defaultValue') }}: {{ agentSpec.default_steps }})
              </span>
            </div>
          </div>
          <p class="mt-2 text-xs text-muted-foreground">
            ctx: {{ selectedProvider?.context_limit_tokens ?? '—' }} · max_tokens: {{ selectedProvider?.max_tokens ?? '—' }}
          </p>
        </section>

        <section class="studio-panel flex flex-col justify-between px-4 py-3.5">
          <div>
            <p class="text-xs text-muted-foreground">{{ t('jobUi.capabilitiesCard') }}</p>
            <p v-if="!isPrimary" class="mt-2 font-mono text-base font-semibold tabular-nums">
              {{ form.allowed_tools.length }}/{{ domainTools.length }} tools ·
              {{ form.allowed_skills.length }}/{{ agentSpec.available_skills.length }} skills
            </p>
            <p v-else class="mt-2 font-mono text-base font-semibold tabular-nums">
              {{ activeSubagents.length }}/{{ flowSubagents.length }} {{ t('jobUi.subagentsAllowed') }}
            </p>
          </div>
          <p class="mt-2 text-xs text-muted-foreground">
            {{
              capabilityEntries.length > 0
                ? `${capabilityEntries.length} capability-routed media tools`
                : `${t('jobUi.allowedCapabilityTags')}: ${allowedCapabilityTags.join(', ')}`
            }}
          </p>
        </section>
      </div>

      <!-- Main 2-Column Studio Workspace -->
      <div class="grid items-start gap-5 xl:grid-cols-[minmax(0,1.4fr)_minmax(360px,0.95fr)]">
        <!-- Left Column: Role Description & System Prompt Studio -->
        <div class="min-w-0 space-y-5">
          <section class="studio-panel space-y-3 p-5" aria-labelledby="agent-description-heading">
            <div class="flex flex-wrap items-start justify-between gap-3">
              <div class="flex items-start gap-2.5">
                <span class="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg bg-studio-violet/10 text-studio-violet">
                  <Bot class="size-4" aria-hidden="true" />
                </span>
                <div>
                  <h2 id="agent-description-heading" class="font-semibold">
                    {{ t('jobUi.agentDescriptionLabel') }}
                  </h2>
                  <p class="mt-0.5 text-xs text-muted-foreground">
                    {{ t('jobUi.agentDescriptionHint') }}
                  </p>
                </div>
              </div>
              <div class="flex items-center gap-2">
                <span class="font-mono text-xs text-muted-foreground">
                  {{ form.description.length }} / 1024
                </span>
                <Button
                  v-if="isDescriptionModified"
                  size="sm"
                  variant="ghost"
                  type="button"
                  @click="form.description = agentSpec.default_description"
                >
                  <RotateCcw class="size-3.5" aria-hidden="true" />{{ t('jobUi.restoreDefault') }}
                </Button>
              </div>
            </div>

            <label class="block">
              <span class="sr-only">{{ t('jobUi.agentDescriptionLabel') }}</span>
              <textarea
                v-model="form.description"
                rows="4"
                maxlength="1024"
                class="admin-textarea text-sm leading-relaxed"
                data-agent-description
                :disabled="saving"
              />
            </label>
          </section>

          <section class="studio-panel space-y-4 p-5" aria-labelledby="agent-system-prompt-heading">
            <div class="flex flex-wrap items-start justify-between gap-3">
              <div class="flex items-start gap-2.5">
                <span class="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg bg-studio-violet/10 text-studio-violet">
                  <FileText class="size-4" aria-hidden="true" />
                </span>
                <div>
                  <div class="flex flex-wrap items-center gap-2">
                    <h2 id="agent-system-prompt-heading" class="font-semibold">
                      {{ t('jobUi.systemPromptLabel') }}
                    </h2>
                    <Badge :variant="isPromptModified ? 'secondary' : 'outline'">
                      {{ isPromptModified ? t('jobUi.customizedBadge') : t('jobUi.defaultBadge') }}
                    </Badge>
                  </div>
                  <p class="mt-0.5 text-xs text-muted-foreground">
                    {{ t('jobUi.systemPromptHint') }}
                  </p>
                </div>
              </div>

              <div class="flex flex-wrap items-center gap-2">
                <div class="inline-flex rounded-lg border border-border/80 bg-surface p-0.5 text-xs">
                  <button
                    type="button"
                    class="inline-flex cursor-pointer items-center gap-1.5 rounded-md px-2.5 py-1 font-medium transition-colors"
                    :class="
                      promptMode === 'edit'
                        ? 'bg-card text-studio-violet shadow-xs'
                        : 'text-muted-foreground hover:text-foreground'
                    "
                    @click="promptMode = 'edit'"
                  >
                    <Pencil class="size-3.5" aria-hidden="true" />{{ t('jobUi.promptModeEdit') }}
                  </button>
                  <button
                    type="button"
                    class="inline-flex cursor-pointer items-center gap-1.5 rounded-md px-2.5 py-1 font-medium transition-colors"
                    :class="
                      promptMode === 'preview'
                        ? 'bg-card text-studio-violet shadow-xs'
                        : 'text-muted-foreground hover:text-foreground'
                    "
                    @click="promptMode = 'preview'"
                  >
                    <Eye class="size-3.5" aria-hidden="true" />{{ t('jobUi.promptModePreview') }}
                  </button>
                  <button
                    type="button"
                    class="inline-flex cursor-pointer items-center gap-1.5 rounded-md px-2.5 py-1 font-medium transition-colors"
                    :class="
                      promptMode === 'split'
                        ? 'bg-card text-studio-violet shadow-xs'
                        : 'text-muted-foreground hover:text-foreground'
                    "
                    @click="promptMode = 'split'"
                  >
                    <Columns2 class="size-3.5" aria-hidden="true" />{{ t('jobUi.promptModeSplit') }}
                  </button>
                </div>

                <Button
                  v-if="isPromptModified"
                  size="sm"
                  variant="ghost"
                  type="button"
                  @click="form.system_prompt = agentSpec.default_system_prompt"
                >
                  <RotateCcw class="size-3.5" aria-hidden="true" />{{ t('jobUi.restoreDefault') }}
                </Button>
              </div>
            </div>

            <div
              v-if="promptMode === 'edit'"
              class="space-y-2"
            >
              <label class="block">
                <span class="sr-only">{{ t('jobUi.systemPromptLabel') }}</span>
                <textarea
                  v-model="form.system_prompt"
                  rows="24"
                  spellcheck="false"
                  class="admin-textarea min-h-[520px] font-mono text-xs leading-relaxed"
                  data-agent-system-prompt
                  :disabled="saving"
                />
              </label>
            </div>

            <div
              v-else-if="promptMode === 'preview'"
              class="prompt-markdown-preview max-h-[720px] min-h-[520px] overflow-y-auto rounded-xl border border-border/70 bg-surface/60 p-5 text-sm leading-relaxed"
              data-agent-prompt-preview
              v-html="renderedPromptHtml"
            />

            <div
              v-else
              class="grid gap-4 lg:grid-cols-2"
            >
              <label class="block">
                <span class="sr-only">{{ t('jobUi.systemPromptLabel') }}</span>
                <textarea
                  v-model="form.system_prompt"
                  rows="24"
                  spellcheck="false"
                  class="admin-textarea min-h-[520px] font-mono text-xs leading-relaxed"
                  data-agent-system-prompt
                  :disabled="saving"
                />
              </label>
              <div
                class="prompt-markdown-preview max-h-[680px] min-h-[520px] overflow-y-auto rounded-xl border border-border/70 bg-surface/60 p-4 text-sm leading-relaxed"
                data-agent-prompt-preview
                v-html="renderedPromptHtml"
              />
            </div>

            <div class="flex flex-wrap items-center justify-between gap-2 border-t pt-3 text-xs text-muted-foreground">
              <span>
                {{ promptLineCount }} {{ t('jobUi.promptLines') }} ·
                {{ form.system_prompt.length }} {{ t('jobUi.promptChars') }}
              </span>
              <span class="font-mono">data/orchestrator-prompts/{{ agentSpec.id }}.md</span>
            </div>
          </section>
        </div>

        <!-- Right Column: LLM Provider, Steps, Tool Providers, Allowed Tools & Skills -->
        <div class="min-w-0 space-y-5">
          <!-- LLM Provider & Execution Budget -->
          <section class="studio-panel space-y-4 p-5" aria-labelledby="agent-llm-execution-heading">
            <div class="flex items-start gap-2.5">
              <span class="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg bg-studio-violet/10 text-studio-violet">
                <Settings2 class="size-4" aria-hidden="true" />
              </span>
              <div>
                <h2 id="agent-llm-execution-heading" class="font-semibold">
                  {{ t('jobUi.llmAndExecutionTitle') }}
                </h2>
                <p class="mt-0.5 text-xs text-muted-foreground">
                  {{ t('jobUi.llmAndExecutionHint') }}
                </p>
              </div>
            </div>

            <label class="block space-y-1.5">
              <span class="text-sm font-medium">{{ t('jobUi.providers') }}</span>
              <select
                v-model="form.provider_ref"
                class="admin-input"
                data-agent-provider
                :disabled="saving"
              >
                <option :value="null">
                  {{ t('jobUi.inheritProvider') }} ({{ fallbackProviderRef }})
                </option>
                <option
                  v-for="option in agentProviderOptions"
                  :key="option.ref"
                  :value="option.ref"
                >
                  {{ formatProviderOptionLabel(option) }}
                </option>
              </select>
            </label>

            <div class="flex flex-wrap items-center gap-1.5 text-xs">
              <span class="text-muted-foreground">{{ t('jobUi.allowedCapabilityTags') }}:</span>
              <Badge v-for="tag in allowedCapabilityTags" :key="tag" variant="outline">
                tag: {{ tag }}
              </Badge>
            </div>

            <div
              v-if="selectedProvider"
              class="space-y-3 rounded-xl border border-border/70 bg-surface/60 p-4 text-xs"
            >
              <div class="flex flex-wrap items-center justify-between gap-2">
                <div class="flex items-center gap-2">
                  <Activity class="size-4 text-studio-violet" aria-hidden="true" />
                  <span class="font-semibold text-foreground">{{ t('jobUi.effectiveProviderDetails') }}</span>
                </div>
                <Button
                  size="sm"
                  variant="ghost"
                  @click="
                    router.push(
                      selectedProvider.source === 'database'
                        ? `/providers/${selectedProvider.ref}`
                        : { name: 'providers', query: { type: 'llm' } },
                    )
                  "
                >
                  <ExternalLink class="size-3.5" aria-hidden="true" />{{ t('jobUi.manageProviders') }}
                </Button>
              </div>

              <dl class="grid grid-cols-[110px_minmax(0,1fr)] gap-x-3 gap-y-2">
                <dt class="text-muted-foreground">Provider</dt>
                <dd class="truncate font-medium text-foreground">
                  {{ selectedProvider.name }}
                  <span class="font-mono text-muted-foreground">({{ selectedProvider.ref }})</span>
                </dd>
                <dt class="text-muted-foreground">Model</dt>
                <dd class="break-all font-mono text-foreground">{{ selectedProvider.model || '—' }}</dd>
                <dt class="text-muted-foreground">Adapter / Source</dt>
                <dd class="font-mono text-foreground">
                  {{ selectedProvider.adapter || 'openai' }} · {{ selectedProvider.source }}
                </dd>
                <template v-if="selectedProvider.base_url">
                  <dt class="text-muted-foreground">Endpoint</dt>
                  <dd class="break-all font-mono text-foreground">
                    {{ redactEndpoint(selectedProvider.base_url) }}
                  </dd>
                </template>
                <dt class="text-muted-foreground">Token limits</dt>
                <dd class="font-mono text-foreground">
                  ctx: {{ selectedProvider.context_limit_tokens ?? '—' }} ·
                  max_tokens: {{ selectedProvider.max_tokens ?? '—' }}
                </dd>
                <template v-if="selectedProvider.reasoning_effort || selectedProvider.temperature_percent != null">
                  <dt class="text-muted-foreground">Inference</dt>
                  <dd class="font-mono text-foreground">
                    <span v-if="selectedProvider.reasoning_effort">
                      reasoning: {{ selectedProvider.reasoning_effort }}
                    </span>
                    <span v-if="selectedProvider.reasoning_effort && selectedProvider.temperature_percent != null"> · </span>
                    <span v-if="selectedProvider.temperature_percent != null">
                      temp: {{ (selectedProvider.temperature_percent / 100).toFixed(2) }}
                    </span>
                  </dd>
                </template>
              </dl>

              <div v-if="selectedProvider.tags?.length" class="flex flex-wrap gap-1.5 border-t border-border/60 pt-2.5">
                <Badge v-for="tag in selectedProvider.tags" :key="tag" variant="secondary">
                  tag: {{ tag }}
                </Badge>
              </div>
            </div>

            <div class="space-y-2 border-t pt-4">
              <div class="flex flex-wrap items-center justify-between gap-2">
                <label for="agent-steps-number" class="text-sm font-medium">
                  {{ t('jobUi.stepsLimit') }} (1 - 100)
                </label>
                <div class="flex items-center gap-2">
                  <Badge variant="outline">
                    {{ t('jobUi.defaultValue') }}: {{ agentSpec.default_steps }}
                  </Badge>
                  <Button
                    v-if="form.steps !== agentSpec.default_steps"
                    size="sm"
                    variant="ghost"
                    type="button"
                    @click="form.steps = agentSpec.default_steps"
                  >
                    <RotateCcw class="size-3.5" aria-hidden="true" />{{ t('jobUi.restoreDefault') }}
                  </Button>
                </div>
              </div>
              <div class="flex items-center gap-3">
                <input
                  v-model.number="form.steps"
                  type="range"
                  min="1"
                  max="100"
                  class="flex-1 accent-primary"
                  :aria-label="t('jobUi.stepsLimit')"
                  :disabled="saving"
                />
                <input
                  id="agent-steps-number"
                  v-model.number="form.steps"
                  type="number"
                  min="1"
                  max="100"
                  class="admin-input w-24 text-center font-mono"
                  data-agent-steps
                  :disabled="saving"
                />
              </div>
              <p class="text-xs text-muted-foreground">{{ t('jobUi.stepsLimitHint') }}</p>
            </div>
          </section>

          <!-- Tool Providers by Capability Tag (e.g. media_creator) -->
          <section
            v-if="capabilityEntries.length > 0"
            class="studio-panel space-y-4 p-5"
            aria-labelledby="agent-tool-providers-heading"
          >
            <div class="flex items-start gap-2.5">
              <span class="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg bg-studio-violet/10 text-studio-violet">
                <Sliders class="size-4" aria-hidden="true" />
              </span>
              <div>
                <h2 id="agent-tool-providers-heading" class="font-semibold">
                  {{ t('jobUi.toolProviders') }}
                </h2>
                <p class="mt-0.5 text-xs text-muted-foreground">
                  {{ t('jobUi.toolProvidersHint') }}
                </p>
              </div>
            </div>

            <div class="space-y-3">
              <div
                v-for="[toolName, requiredTag] in capabilityEntries"
                :key="toolName"
                class="space-y-2 rounded-xl border border-border/70 bg-surface/60 p-3.5 text-xs"
              >
                <div class="flex flex-wrap items-center justify-between gap-2">
                  <div class="flex items-center gap-2">
                    <code class="rounded bg-muted px-2 py-0.5 font-semibold text-foreground">{{ toolName }}</code>
                    <Badge variant="outline">tag: {{ requiredTag }}</Badge>
                  </div>
                  <span
                    v-if="agentSpec.effective_tool_providers?.[toolName]"
                    class="font-mono text-muted-foreground"
                  >
                    {{ t('jobUi.effectiveToolProvider') }}:
                    <strong class="text-foreground">{{ agentSpec.effective_tool_providers[toolName] }}</strong>
                  </span>
                </div>
                <select
                  v-model="form.tool_providers[toolName]"
                  :data-tool-provider="toolName"
                  class="admin-input h-9 text-xs"
                  :disabled="saving"
                >
                  <option value="">
                    {{ t('jobUi.autoToolProvider') }} ({{ requiredTag }})
                  </option>
                  <option
                    v-for="option in providerOptionsForTag(providerOptions, requiredTag)"
                    :key="option.ref"
                    :value="option.ref"
                  >
                    {{ formatProviderOptionLabel(option) }}
                  </option>
                </select>
              </div>
            </div>
          </section>

          <!-- Sub-Agent Allowed Tools OR Root Agent Delegated Sub-Agents -->
          <section
            v-if="!isPrimary && domainTools.length > 0"
            class="studio-panel space-y-4 p-5"
            aria-labelledby="agent-allowed-tools-heading"
          >
            <div class="flex flex-wrap items-start justify-between gap-2">
              <div class="flex items-start gap-2.5">
                <span class="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg bg-studio-violet/10 text-studio-violet">
                  <Wrench class="size-4" aria-hidden="true" />
                </span>
                <div>
                  <div class="flex items-center gap-2">
                    <h2 id="agent-allowed-tools-heading" class="font-semibold">
                      {{ t('jobUi.toolsAllowed') }}
                    </h2>
                    <Badge variant="secondary" class="tabular-nums">
                      {{ form.allowed_tools.length }}/{{ domainTools.length }}
                    </Badge>
                  </div>
                  <p class="mt-0.5 text-xs text-muted-foreground">
                    {{ t('jobUi.toolsAllowedHint') }}
                  </p>
                </div>
              </div>

              <div class="flex items-center gap-1.5">
                <Button
                  size="sm"
                  variant="ghost"
                  type="button"
                  :disabled="saving || form.allowed_tools.length === domainTools.length"
                  @click="selectAllTools"
                >
                  {{ t('jobUi.selectAll') }}
                </Button>
                <Button
                  v-if="!sameStringArray(form.allowed_tools, defaultDomainTools)"
                  size="sm"
                  variant="ghost"
                  type="button"
                  :disabled="saving"
                  @click="resetDefaultTools"
                >
                  <RotateCcw class="size-3.5" aria-hidden="true" />{{ t('jobUi.restoreDefault') }}
                </Button>
              </div>
            </div>

            <div class="grid gap-2 sm:grid-cols-2">
              <label
                v-for="tool in domainTools"
                :key="tool"
                class="flex cursor-pointer items-center justify-between gap-2 rounded-xl border p-3 text-xs transition-colors"
                :class="
                  form.allowed_tools.includes(tool)
                    ? 'border-studio-violet/35 bg-studio-violet/5'
                    : 'border-border/70 bg-surface/40 opacity-75 hover:opacity-100'
                "
              >
                <div class="flex min-w-0 items-center gap-2.5">
                  <input
                    type="checkbox"
                    class="size-4 shrink-0 accent-primary"
                    :checked="form.allowed_tools.includes(tool)"
                    :disabled="saving"
                    @change="toggleTool(tool, ($event.target as HTMLInputElement).checked)"
                  />
                  <code class="truncate font-medium">{{ tool }}</code>
                </div>
              </label>
            </div>
          </section>

          <section
            v-else-if="isPrimary && flowSubagents.length > 0"
            class="studio-panel space-y-4 p-5"
            aria-labelledby="root-delegation-heading"
          >
            <div class="flex items-start justify-between gap-2">
              <div class="flex items-start gap-2.5">
                <span class="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg bg-studio-violet/10 text-studio-violet">
                  <Workflow class="size-4" aria-hidden="true" />
                </span>
                <div>
                  <div class="flex items-center gap-2">
                    <h2 id="root-delegation-heading" class="font-semibold">
                      {{ t('jobUi.rootDelegationTitle') }}
                    </h2>
                    <Badge variant="secondary" class="tabular-nums">
                      {{ activeSubagents.length }}/{{ flowSubagents.length }}
                    </Badge>
                  </div>
                  <p class="mt-0.5 text-xs text-muted-foreground">
                    {{ t('jobUi.rootDelegationHint') }}
                  </p>
                </div>
              </div>
            </div>

            <ul class="divide-y divide-border/70 rounded-xl border border-border/70 bg-surface/40">
              <li
                v-for="sub in flowSubagents"
                :key="sub.id"
                class="flex items-center justify-between gap-3 p-3.5"
              >
                <div class="min-w-0 flex-1 space-y-1">
                  <div class="flex flex-wrap items-center gap-2">
                    <code class="rounded bg-muted px-2 py-0.5 text-xs font-semibold">{{ sub.id }}</code>
                    <Badge :variant="sub.enabled ? 'success' : 'outline'">
                      {{ sub.enabled ? t('jobUi.on') : t('jobUi.off') }}
                    </Badge>
                    <Badge variant="outline">model: {{ sub.effective_model }}</Badge>
                  </div>
                  <p class="line-clamp-2 text-xs text-muted-foreground">{{ sub.description }}</p>
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  @click="switchAgent(sub.id)"
                >
                  {{ t('jobUi.configureAgent') }}<ArrowRight class="size-3.5" aria-hidden="true" />
                </Button>
              </li>
            </ul>
          </section>

          <!-- Sub-Agent Allowed Skills -->
          <section
            v-if="!isPrimary && agentSpec.available_skills.length > 0"
            class="studio-panel space-y-4 p-5"
            aria-labelledby="agent-allowed-skills-heading"
          >
            <div class="flex flex-wrap items-start justify-between gap-2">
              <div class="flex items-start gap-2.5">
                <span class="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg bg-studio-violet/10 text-studio-violet">
                  <BookOpen class="size-4" aria-hidden="true" />
                </span>
                <div>
                  <div class="flex items-center gap-2">
                    <h2 id="agent-allowed-skills-heading" class="font-semibold">
                      {{ t('jobUi.skillsAllowed') }}
                    </h2>
                    <Badge variant="secondary" class="tabular-nums">
                      {{ form.allowed_skills.length }}/{{ agentSpec.available_skills.length }}
                    </Badge>
                  </div>
                  <p class="mt-0.5 text-xs text-muted-foreground">
                    {{ t('jobUi.skillsAllowedHint') }}
                  </p>
                </div>
              </div>

              <div class="flex items-center gap-1.5">
                <Button
                  size="sm"
                  variant="ghost"
                  type="button"
                  :disabled="saving || form.allowed_skills.length === agentSpec.available_skills.length"
                  @click="selectAllSkills"
                >
                  {{ t('jobUi.selectAll') }}
                </Button>
                <Button
                  v-if="!sameStringArray(form.allowed_skills, agentSpec.default_allowed_skills ?? [])"
                  size="sm"
                  variant="ghost"
                  type="button"
                  :disabled="saving"
                  @click="resetDefaultSkills"
                >
                  <RotateCcw class="size-3.5" aria-hidden="true" />{{ t('jobUi.restoreDefault') }}
                </Button>
              </div>
            </div>

            <div class="grid gap-2 sm:grid-cols-2">
              <label
                v-for="skill in agentSpec.available_skills"
                :key="skill"
                class="flex cursor-pointer items-center justify-between gap-2 rounded-xl border p-3 text-xs transition-colors"
                :class="
                  form.allowed_skills.includes(skill)
                    ? 'border-studio-violet/35 bg-studio-violet/5'
                    : 'border-border/70 bg-surface/40 opacity-75 hover:opacity-100'
                "
              >
                <div class="flex min-w-0 items-center gap-2.5">
                  <input
                    type="checkbox"
                    class="size-4 shrink-0 accent-primary"
                    :checked="form.allowed_skills.includes(skill)"
                    :disabled="saving"
                    @change="toggleSkill(skill, ($event.target as HTMLInputElement).checked)"
                  />
                  <code class="truncate font-medium">{{ skill }}</code>
                </div>
              </label>
            </div>
          </section>
        </div>
      </div>
    </template>
  </main>
</template>

<style scoped>
.prompt-markdown-preview :deep(h1),
.prompt-markdown-preview :deep(h2),
.prompt-markdown-preview :deep(h3),
.prompt-markdown-preview :deep(h4) {
  font-weight: 650;
  margin-top: 1.1rem;
  margin-bottom: 0.5rem;
  color: var(--foreground);
}
.prompt-markdown-preview :deep(h1) { font-size: 1.25rem; }
.prompt-markdown-preview :deep(h2) { font-size: 1.1rem; }
.prompt-markdown-preview :deep(h3) { font-size: 1rem; }
.prompt-markdown-preview :deep(p) {
  margin: 0.5rem 0;
}
.prompt-markdown-preview :deep(ul),
.prompt-markdown-preview :deep(ol) {
  padding-left: 1.25rem;
  margin: 0.5rem 0;
}
.prompt-markdown-preview :deep(ul) { list-style: disc; }
.prompt-markdown-preview :deep(ol) { list-style: decimal; }
.prompt-markdown-preview :deep(code) {
  font-size: 0.75rem;
  padding: 0.1rem 0.35rem;
  border-radius: 0.25rem;
  background: var(--muted);
}
.prompt-markdown-preview :deep(pre) {
  margin: 0.75rem 0;
  padding: 0.75rem;
  border-radius: 0.5rem;
  border: 1px solid var(--border);
  background: var(--card);
  overflow-x: auto;
}
.prompt-markdown-preview :deep(pre code) {
  padding: 0;
  background: transparent;
}
</style>

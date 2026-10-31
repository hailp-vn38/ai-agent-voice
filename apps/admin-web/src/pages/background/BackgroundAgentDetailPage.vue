<script setup lang="ts">
import {
  ArrowLeft,
  ArrowRight,
  BookOpen,
  Bot,
  Columns2,
  Cpu,
  Eye,
  ExternalLink,
  LayoutDashboard,
  Pencil,
  Plus,
  RefreshCw,
  RotateCcw,
  Save,
  Sliders,
  Trash2,
  Workflow,
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
  JobSkillDetail,
  JobSkillSummary,
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
import JobSkillEditorModal from './JobSkillEditorModal.vue'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()

const agentId = computed(() => String(route.params.agentId ?? ''))
const settings = ref<BackgroundSettings | null>(null)
const loading = ref(true)
const saving = ref(false)
const deleting = ref(false)
const error = ref('')
const notice = ref('')
const unavailable = ref(false)
const promptMode = ref<'edit' | 'preview' | 'split'>('edit')
const activeSection = ref<'overview' | 'model' | 'prompt' | 'tools' | 'skills'>('overview')

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
const skillsCatalog = computed(() => settings.value?.skills ?? [])
const dailyFlowAgents = computed(() => agentsCatalog.value.filter(a => a.flow_id === 'daily'))
const wikiFlowAgents = computed(() => agentsCatalog.value.filter(a => a.flow_id === 'wiki'))

const skillModalOpen = ref(false)
const skillModalMode = ref<'create' | 'edit'>('create')
const activeSkillId = ref<string | null>(null)

function findSkillSummary(id: string): JobSkillSummary | undefined {
  return skillsCatalog.value.find(s => s.id === id)
}

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

async function onSkillSaved(saved: JobSkillDetail) {
  const createdNew = skillModalMode.value === 'create'
  skillModalOpen.value = false
  try {
    const config = await agentJobsApi.settings()
    settings.value = config
    if (createdNew && !form.value.allowed_skills.includes(saved.id)) {
      form.value.allowed_skills = [...form.value.allowed_skills, saved.id]
    }
    notice.value = t('jobUi.skillSaved')
  } catch (cause) {
    error.value = formatApiError(cause)
  }
}

async function onSkillDeleted(deletedId: string) {
  skillModalOpen.value = false
  form.value.allowed_skills = form.value.allowed_skills.filter(s => s !== deletedId)
  try {
    const config = await agentJobsApi.settings()
    settings.value = config
    notice.value = t('jobUi.skillDeleted')
  } catch (cause) {
    error.value = formatApiError(cause)
  }
}

const agentSpec = computed<BackgroundAgentConfig | null>(
  () => agentsCatalog.value.find(a => a.id === agentId.value) ?? null,
)
const isPrimary = computed(() => agentSpec.value?.mode === 'primary')
const isCustomSubagent = computed(() => agentSpec.value?.is_builtin === false)
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

const anchorItems = computed(() => {
  const base: Array<{
    key: 'overview' | 'model' | 'prompt' | 'tools' | 'skills'
    label: string
    icon: unknown
  }> = [
    { key: 'overview', label: t('jobUi.agentNavOverview'), icon: LayoutDashboard },
    { key: 'model', label: t('jobUi.agentNavModel'), icon: Cpu },
    { key: 'prompt', label: t('jobUi.agentNavPrompt'), icon: Pencil },
    { key: 'tools', label: t('jobUi.agentNavTools'), icon: Sliders },
  ]
  if (!isPrimary.value) {
    base.push({ key: 'skills', label: t('jobUi.agentNavSkills'), icon: BookOpen })
  }
  return base
})

function scrollToSection(key: 'overview' | 'model' | 'prompt' | 'tools' | 'skills') {
  activeSection.value = key
  if (typeof document !== 'undefined') {
    const el = document.getElementById(`agent-section-${key}`)
    if (el && typeof el.scrollIntoView === 'function') {
      el.scrollIntoView({ behavior: 'smooth', block: 'center' })
    }
  }
}

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
      skills: result.skills ?? settings.value.skills,
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

async function deleteCustomSubagent() {
  const spec = agentSpec.value
  if (!spec || spec.is_builtin !== false || !settings.value || deleting.value) return
  if (typeof window !== 'undefined' && typeof window.confirm === 'function') {
    const confirmed = window.confirm(t('jobUi.subagentDeleteConfirm', { id: spec.id }))
    if (!confirmed) return
  }
  deleting.value = true
  error.value = ''
  notice.value = ''
  try {
    await agentJobsApi.deleteSubagent(spec.id, settings.value.revision)
    await router.push({ name: 'jobs-settings-agents' })
  } catch (cause) {
    error.value = formatApiError(cause)
    if (isApiError(cause) && cause.status === 409) {
      await load()
    }
  } finally {
    deleting.value = false
  }
}

watch(
  agentId,
  () => {
    notice.value = ''
    error.value = ''
    activeSection.value = 'overview'
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
  <section class="space-y-4">
    <DetailHeader
      :title="agentSpec ? agentSpec.id : agentId || t('jobUi.agentModalTitle')"
      :back-label="t('jobUi.agentsPageTitle')"
      @back="router.push({ name: 'jobs-settings-agents' })"
    >
      <template #icon>
        <span
          class="flex size-9 shrink-0 items-center justify-center rounded-xl border border-studio-violet/20 bg-studio-violet/10 text-studio-violet"
        >
          <Bot v-if="isPrimary" class="size-4.5" aria-hidden="true" />
          <Cpu v-else class="size-4.5" aria-hidden="true" />
        </span>
      </template>
      <template v-if="agentSpec" #details>
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span class="font-semibold uppercase tracking-wider text-studio-violet">
            {{ t('jobUi.agentDetailEyebrow') }}
          </span>
          <span aria-hidden="true">·</span>
          <Badge :variant="isPrimary ? 'default' : 'secondary'">
            {{ isPrimary ? t('jobUi.primaryAgent') : t('jobUi.subagentRole') }}
          </Badge>
          <span>
            {{
              agentSpec.flow_id === 'daily'
                ? t('jobUi.flowDailyShort')
                : t('jobUi.flowWikiShort')
            }}
          </span>
          <template v-if="agentSpec.parent_id">
            <span aria-hidden="true">·</span>
            <span>
              {{ t('jobUi.parentCoordinator') }}:
              <code class="font-mono font-semibold text-foreground">{{ agentSpec.parent_id }}</code>
            </span>
          </template>
          <span aria-hidden="true">·</span>
          <Badge :variant="form.enabled ? 'success' : 'outline'">
            {{ form.enabled ? t('jobUi.on') : t('jobUi.off') }}
          </Badge>
          <Badge variant="outline">
            {{ isCustomSubagent ? t('jobUi.subagentCustomBadge') : t('jobUi.subagentBuiltinBadge') }}
          </Badge>
          <Badge v-if="!isCustomSubagent" :variant="isCustomized ? 'secondary' : 'outline'">
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
          :disabled="loading || saving || deleting"
          @click="router.push({ name: 'providers', query: { type: 'llm' } })"
        >
          <ExternalLink class="size-4" aria-hidden="true" />
          {{ t('jobUi.manageProviders') }}
        </Button>
        <Button variant="outline" size="sm" :disabled="loading || saving || deleting" @click="load">
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" aria-hidden="true" />
          {{ t('jobUi.refresh') }}
        </Button>
        <Button
          v-if="agentSpec && !isCustomSubagent"
          variant="outline"
          size="sm"
          :disabled="loading || saving || !isCustomized"
          data-reset-agent-default
          @click="resetToDefault"
        >
          <RotateCcw class="size-4" aria-hidden="true" />
          {{ t('jobUi.resetAgentDefault') }}
        </Button>
        <Button
          v-if="agentSpec && isCustomSubagent"
          variant="outline"
          size="sm"
          class="border-danger/40 text-danger hover:bg-danger/10"
          :disabled="loading || saving || deleting"
          data-delete-subagent
          data-delete-custom-subagent
          @click="deleteCustomSubagent"
        >
          <Trash2 class="size-4" aria-hidden="true" />
          {{ t('jobUi.subagentDelete') }}
        </Button>
        <Button
          v-if="agentSpec"
          size="sm"
          :disabled="!dirty || saving || deleting"
          data-save-agent-config
          @click="save"
        >
          <Save class="size-4" aria-hidden="true" />
          {{ t('jobUi.save') }}
        </Button>
      </template>
    </DetailHeader>

    <div
      v-if="error"
      role="alert"
      class="rounded-lg border border-danger/30 bg-danger/5 p-3.5 text-sm text-danger"
    >
      <p class="font-medium">{{ unavailable ? t('jobUi.backendPending') : t('jobUi.loadFailed') }}</p>
      <p class="mt-1 break-words">{{ error }}</p>
    </div>
    <p
      v-if="notice"
      role="status"
      class="rounded-lg border border-success/30 bg-success/5 p-3 text-sm text-success-foreground"
    >
      {{ notice }}
    </p>

    <div
      v-if="loading && !settings"
      class="studio-panel p-6 text-center text-sm text-muted-foreground"
      role="status"
    >
      {{ t('common.loading') }}
    </div>

    <section v-else-if="!loading && !agentSpec" class="studio-panel space-y-3 p-6 text-center">
      <h2 class="text-lg font-semibold">{{ t('jobUi.agentNotFound') }}</h2>
      <p class="text-xs text-muted-foreground">{{ t('jobUi.agentNotFoundHint') }}</p>
      <Button variant="outline" size="sm" @click="router.push({ name: 'jobs-settings-agents' })">
        <ArrowLeft class="size-4" aria-hidden="true" />
        {{ t('jobUi.agentsPageTitle') }}
      </Button>
    </section>

    <template v-else-if="agentSpec">
      <!-- Detail Grid: Sticky Anchor Nav (Left) + Stacked Groupboxes (Right) -->
      <div class="grid items-start gap-4 lg:grid-cols-[185px_minmax(0,1fr)]">
        <!-- Sticky Anchor Nav + Quick Flow Agent Switcher -->
        <aside class="space-y-3 lg:sticky lg:top-4">
          <nav
            class="flex gap-1 overflow-x-auto lg:flex-col"
            aria-label="Agent configuration sections"
            data-anchor-nav
          >
            <button
              v-for="item in anchorItems"
              :key="item.key"
              type="button"
              :data-tab="item.key"
              class="inline-flex cursor-pointer items-center gap-2 rounded-lg px-3 py-2 text-left text-xs transition-colors whitespace-nowrap"
              :class="
                activeSection === item.key
                  ? 'bg-studio-violet/15 font-semibold text-studio-violet'
                  : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground'
              "
              @click="scrollToSection(item.key)"
            >
              <component :is="item.icon" class="size-3.5 shrink-0" aria-hidden="true" />
              <span>{{ item.label }}</span>
            </button>
          </nav>

          <!-- Compact Quick Agent Switcher -->
          <div
            v-if="agentsCatalog.length > 1"
            class="hidden rounded-xl border border-border/70 bg-card p-2.5 lg:block"
            :aria-label="t('jobUi.quickSwitchAgent')"
          >
            <div class="mb-1.5 px-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
              {{ t('jobUi.flowDailyLabel') }}
            </div>
            <div class="space-y-0.5">
              <button
                v-for="item in dailyFlowAgents"
                :key="item.id"
                type="button"
                class="flex w-full cursor-pointer items-center gap-1.5 rounded-md px-2 py-1 text-left font-mono text-[11px] transition-colors"
                :class="
                  item.id === agentSpec.id
                    ? 'bg-studio-violet/15 font-semibold text-studio-violet'
                    : 'text-muted-foreground hover:bg-accent/40 hover:text-foreground'
                "
                @click="switchAgent(item.id)"
              >
                <span
                  class="size-1.5 shrink-0 rounded-full"
                  :class="item.enabled ? 'bg-success' : 'bg-muted-foreground/45'"
                  aria-hidden="true"
                />
                <span class="truncate">{{ item.id }}</span>
              </button>
            </div>

            <template v-if="wikiFlowAgents.length > 0">
              <div class="mt-2.5 mb-1.5 border-t border-border/60 pt-2 px-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                {{ t('jobUi.flowWikiLabel') }}
              </div>
              <div class="space-y-0.5">
                <button
                  v-for="item in wikiFlowAgents"
                  :key="item.id"
                  type="button"
                  class="flex w-full cursor-pointer items-center gap-1.5 rounded-md px-2 py-1 text-left font-mono text-[11px] transition-colors"
                  :class="
                    item.id === agentSpec.id
                      ? 'bg-studio-violet/15 font-semibold text-studio-violet'
                      : 'text-muted-foreground hover:bg-accent/40 hover:text-foreground'
                  "
                  @click="switchAgent(item.id)"
                >
                  <span
                    class="size-1.5 shrink-0 rounded-full"
                    :class="item.enabled ? 'bg-success' : 'bg-muted-foreground/45'"
                    aria-hidden="true"
                  />
                  <span class="truncate">{{ item.id }}</span>
                </button>
              </div>
            </template>
          </div>
        </aside>

        <!-- Stacked Form Groupboxes -->
        <div class="min-w-0 space-y-4">
          <!-- Groupbox 1: Overview & Status -->
          <section
            id="agent-section-overview"
            class="studio-panel space-y-3.5 p-4"
            aria-labelledby="agent-overview-heading"
          >
            <div class="flex flex-wrap items-center justify-between gap-3">
              <div>
                <h2 id="agent-overview-heading" class="text-sm font-semibold">
                  {{ t('jobUi.agentOverviewBoxTitle') }}
                </h2>
                <p class="mt-0.5 text-xs text-muted-foreground">
                  {{ t('jobUi.agentOverviewBoxSubtitle') }}
                </p>
              </div>

              <label
                class="flex items-center gap-2 text-xs font-medium"
                :class="isPrimary ? 'cursor-not-allowed opacity-70' : 'cursor-pointer'"
                :title="isPrimary ? t('jobUi.primaryAlwaysActive') : t('jobUi.subagentToggleHint')"
              >
                <span>{{ t('jobUi.agentEnableToggle') }}</span>
                <span class="relative inline-flex shrink-0 items-center">
                  <input
                    v-model="form.enabled"
                    type="checkbox"
                    role="switch"
                    :aria-label="t('jobUi.agentStatusCard')"
                    :data-agent-toggle="agentSpec.id"
                    class="peer sr-only"
                    :disabled="isPrimary || saving"
                  />
                  <span
                    aria-hidden="true"
                    class="h-5 w-9 rounded-full bg-muted-foreground/40 transition-colors peer-checked:bg-primary peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-ring peer-disabled:opacity-40"
                  />
                  <span
                    aria-hidden="true"
                    class="pointer-events-none absolute left-[2px] size-4 rounded-full bg-white shadow transition-transform peer-checked:translate-x-4"
                  />
                </span>
              </label>
            </div>

            <div class="grid gap-3.5 sm:grid-cols-2">
              <div class="space-y-1.5">
                <label class="block text-xs font-semibold">Agent ID</label>
                <input
                  class="admin-input h-9 font-mono text-xs"
                  :value="agentSpec.id"
                  readonly
                />
              </div>

              <div class="space-y-1.5">
                <label class="block text-xs font-semibold">Flow</label>
                <input
                  class="admin-input h-9 text-xs"
                  :value="
                    agentSpec.flow_id === 'daily'
                      ? t('jobUi.flowDailyLabel')
                      : t('jobUi.flowWikiLabel')
                  "
                  readonly
                />
              </div>
            </div>

            <div class="space-y-1.5">
              <div class="flex flex-wrap items-center justify-between gap-2">
                <label class="text-xs font-semibold">
                  {{ t('jobUi.agentDescriptionLabel') }}
                </label>
                <div class="flex items-center gap-2">
                  <span class="font-mono text-[11px] text-muted-foreground">
                    {{ form.description.length }} / 1024
                  </span>
                  <Button
                    v-if="!isCustomSubagent && isDescriptionModified"
                    size="sm"
                    variant="ghost"
                    type="button"
                    class="h-6 px-2 text-xs"
                    @click="form.description = agentSpec.default_description"
                  >
                    <RotateCcw class="size-3" aria-hidden="true" />
                    {{ t('jobUi.restoreDefault') }}
                  </Button>
                </div>
              </div>
              <textarea
                v-model="form.description"
                rows="3"
                maxlength="1024"
                class="admin-textarea text-xs leading-relaxed"
                data-agent-description
                :disabled="saving"
              />
            </div>
          </section>

          <!-- Groupbox 2: Model & Execution -->
          <section
            id="agent-section-model"
            class="studio-panel space-y-3.5 p-4"
            aria-labelledby="agent-llm-execution-heading"
          >
            <div class="flex flex-wrap items-center justify-between gap-3">
              <div>
                <h2 id="agent-llm-execution-heading" class="text-sm font-semibold">
                  {{ t('jobUi.agentModelBoxTitle') }}
                </h2>
                <p class="mt-0.5 text-xs text-muted-foreground">
                  {{ t('jobUi.agentModelBoxSubtitle') }}
                </p>
              </div>
              <Badge variant="default">
                {{ selectedProvider?.model ?? agentSpec.effective_model }}
              </Badge>
            </div>

            <div class="grid gap-3.5 sm:grid-cols-2">
              <label class="block space-y-1.5">
                <span class="block text-xs font-semibold">{{ t('jobUi.providers') }}</span>
                <select
                  v-model="form.provider_ref"
                  class="admin-input h-9 text-xs"
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

              <div class="space-y-1.5">
                <div class="flex items-center justify-between gap-2">
                  <label for="agent-steps-number" class="text-xs font-semibold">
                    {{ t('jobUi.stepsLimit') }} · {{ form.steps }}
                  </label>
                  <div class="flex items-center gap-1.5">
                    <Badge v-if="!isCustomSubagent" variant="outline">
                      {{ t('jobUi.defaultValue') }}: {{ agentSpec.default_steps }}
                    </Badge>
                    <Button
                      v-if="!isCustomSubagent && form.steps !== agentSpec.default_steps"
                      size="sm"
                      variant="ghost"
                      type="button"
                      class="h-6 px-2 text-xs"
                      @click="form.steps = agentSpec.default_steps"
                    >
                      <RotateCcw class="size-3" aria-hidden="true" />
                      {{ t('jobUi.restoreDefault') }}
                    </Button>
                  </div>
                </div>
                <div class="flex items-center gap-2.5">
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
                    class="admin-input h-9 w-20 text-center font-mono text-xs"
                    data-agent-steps
                    :disabled="saving"
                  />
                </div>
              </div>
            </div>

            <!-- Compact Effective Provider Metadata -->
            <div class="flex flex-wrap items-center justify-between gap-2 border-t border-border/60 pt-3 text-xs">
              <div class="flex flex-wrap items-center gap-1.5">
                <span class="text-muted-foreground">{{ t('jobUi.allowedCapabilityTags') }}:</span>
                <Badge v-for="tag in allowedCapabilityTags" :key="tag" variant="outline">
                  tag: {{ tag }}
                </Badge>
              </div>

              <div v-if="selectedProvider" class="flex flex-wrap items-center gap-2 font-mono text-[11px] text-muted-foreground">
                <span>
                  {{ selectedProvider.name }} ({{ selectedProvider.ref }})
                </span>
                <span v-if="selectedProvider.base_url">
                  · {{ redactEndpoint(selectedProvider.base_url) }}
                </span>
                <span>
                  · ctx: {{ selectedProvider.context_limit_tokens ?? '—' }} · max_tokens: {{ selectedProvider.max_tokens ?? '—' }}
                </span>
              </div>
            </div>
          </section>

          <!-- Groupbox 3: System Prompt -->
          <section
            id="agent-section-prompt"
            class="studio-panel space-y-3 p-4"
            aria-labelledby="agent-system-prompt-heading"
          >
            <div class="flex flex-wrap items-center justify-between gap-3">
              <div>
                <div class="flex flex-wrap items-center gap-2">
                  <h2 id="agent-system-prompt-heading" class="text-sm font-semibold">
                    {{ t('jobUi.systemPromptLabel') }}
                  </h2>
                  <Badge v-if="!isCustomSubagent" :variant="isPromptModified ? 'secondary' : 'outline'">
                    {{ isPromptModified ? t('jobUi.customizedBadge') : t('jobUi.defaultBadge') }}
                  </Badge>
                </div>
                <p class="mt-0.5 text-xs text-muted-foreground">
                  {{ t('jobUi.systemPromptHint') }}
                </p>
              </div>
              <Badge variant="outline">Markdown</Badge>
            </div>

            <div class="flex flex-wrap items-center justify-between gap-2 border-b border-border/70 pb-2.5">
              <span class="font-mono text-xs text-muted-foreground">
                SYSTEM_PROMPT.md · data/orchestrator-prompts/{{ agentSpec.id }}.md
              </span>

              <div class="flex flex-wrap items-center gap-2">
                <span class="font-mono text-[11px] text-muted-foreground tabular-nums">
                  {{ promptLineCount }} {{ t('jobUi.promptLines') }} · {{ form.system_prompt.length }} {{ t('jobUi.promptChars') }}
                </span>

                <div class="inline-flex rounded-lg border border-border/80 bg-surface p-0.5 text-xs">
                  <button
                    type="button"
                    class="inline-flex cursor-pointer items-center gap-1 rounded-md px-2.5 py-1 font-medium transition-colors"
                    :class="
                      promptMode === 'edit'
                        ? 'bg-card text-studio-violet shadow-xs'
                        : 'text-muted-foreground hover:text-foreground'
                    "
                    @click="promptMode = 'edit'"
                  >
                    <Pencil class="size-3" aria-hidden="true" />
                    {{ t('jobUi.promptModeEdit') }}
                  </button>
                  <button
                    type="button"
                    class="inline-flex cursor-pointer items-center gap-1 rounded-md px-2.5 py-1 font-medium transition-colors"
                    :class="
                      promptMode === 'preview'
                        ? 'bg-card text-studio-violet shadow-xs'
                        : 'text-muted-foreground hover:text-foreground'
                    "
                    @click="promptMode = 'preview'"
                  >
                    <Eye class="size-3" aria-hidden="true" />
                    {{ t('jobUi.promptModePreview') }}
                  </button>
                  <button
                    type="button"
                    class="inline-flex cursor-pointer items-center gap-1 rounded-md px-2.5 py-1 font-medium transition-colors"
                    :class="
                      promptMode === 'split'
                        ? 'bg-card text-studio-violet shadow-xs'
                        : 'text-muted-foreground hover:text-foreground'
                    "
                    @click="promptMode = 'split'"
                  >
                    <Columns2 class="size-3" aria-hidden="true" />
                    {{ t('jobUi.promptModeSplit') }}
                  </button>
                </div>

                <Button
                  v-if="!isCustomSubagent && isPromptModified"
                  size="sm"
                  variant="ghost"
                  type="button"
                  class="h-7 px-2 text-xs"
                  @click="form.system_prompt = agentSpec.default_system_prompt"
                >
                  <RotateCcw class="size-3" aria-hidden="true" />
                  {{ t('jobUi.restoreDefault') }}
                </Button>
              </div>
            </div>

            <div v-if="promptMode === 'edit'">
              <label class="block">
                <span class="sr-only">{{ t('jobUi.systemPromptLabel') }}</span>
                <textarea
                  v-model="form.system_prompt"
                  rows="16"
                  spellcheck="false"
                  class="admin-textarea min-h-[300px] font-mono text-xs leading-relaxed"
                  data-agent-system-prompt
                  :disabled="saving"
                />
              </label>
            </div>

            <div
              v-else-if="promptMode === 'preview'"
              class="prompt-markdown-preview max-h-[540px] min-h-[300px] overflow-y-auto rounded-xl border border-border/70 bg-surface/60 p-4 text-xs leading-relaxed"
              data-agent-prompt-preview
              v-html="renderedPromptHtml"
            />

            <div v-else class="grid gap-3 lg:grid-cols-2">
              <label class="block">
                <span class="sr-only">{{ t('jobUi.systemPromptLabel') }}</span>
                <textarea
                  v-model="form.system_prompt"
                  rows="16"
                  spellcheck="false"
                  class="admin-textarea min-h-[300px] font-mono text-xs leading-relaxed"
                  data-agent-system-prompt
                  :disabled="saving"
                />
              </label>
              <div
                class="prompt-markdown-preview max-h-[540px] min-h-[300px] overflow-y-auto rounded-xl border border-border/70 bg-surface/60 p-4 text-xs leading-relaxed"
                data-agent-prompt-preview
                v-html="renderedPromptHtml"
              />
            </div>
          </section>

          <!-- Groupbox 4: Tools & Permissions (Subagent) OR Delegated Sub-Agents (Root Agent) -->
          <section
            id="agent-section-tools"
            class="studio-panel space-y-3.5 p-4"
            aria-labelledby="agent-allowed-tools-heading"
          >
            <template v-if="!isPrimary">
              <div class="flex flex-wrap items-center justify-between gap-2">
                <div>
                  <h2 id="agent-allowed-tools-heading" class="text-sm font-semibold">
                    {{ t('jobUi.agentToolsBoxTitle') }}
                  </h2>
                  <p class="mt-0.5 text-xs text-muted-foreground">
                    {{ t('jobUi.agentToolsBoxSubtitle') }}
                  </p>
                </div>

                <div class="flex items-center gap-1.5">
                  <Badge variant="secondary" class="tabular-nums">
                    {{ form.allowed_tools.length }}/{{ domainTools.length }} tools
                  </Badge>
                  <Button
                    v-if="domainTools.length > 0"
                    size="sm"
                    variant="ghost"
                    type="button"
                    class="h-7 px-2 text-xs"
                    :disabled="saving || form.allowed_tools.length === domainTools.length"
                    @click="selectAllTools"
                  >
                    {{ t('jobUi.selectAll') }}
                  </Button>
                  <Button
                    v-if="!isCustomSubagent && !sameStringArray(form.allowed_tools, defaultDomainTools)"
                    size="sm"
                    variant="ghost"
                    type="button"
                    class="h-7 px-2 text-xs"
                    :disabled="saving"
                    @click="resetDefaultTools"
                  >
                    <RotateCcw class="size-3" aria-hidden="true" />
                    {{ t('jobUi.restoreDefault') }}
                  </Button>
                </div>
              </div>

              <div v-if="domainTools.length > 0" class="flex flex-wrap gap-2">
                <label
                  v-for="tool in domainTools"
                  :key="tool"
                  class="inline-flex cursor-pointer items-center gap-2 rounded-lg border px-2.5 py-1.5 text-xs transition-colors"
                  :class="
                    form.allowed_tools.includes(tool)
                      ? 'border-studio-violet/40 bg-studio-violet/10 font-medium text-studio-violet'
                      : 'border-border/75 bg-surface/50 text-foreground hover:bg-accent/40'
                  "
                >
                  <input
                    type="checkbox"
                    class="size-3.5 shrink-0 accent-primary"
                    :checked="form.allowed_tools.includes(tool)"
                    :disabled="saving"
                    @change="toggleTool(tool, ($event.target as HTMLInputElement).checked)"
                  />
                  <code class="font-mono text-[11px]">{{ tool }}</code>
                </label>
              </div>

              <!-- Capability-routed Tool Providers (e.g. media_creator / media_processor) -->
              <div
                v-if="capabilityEntries.length > 0"
                class="space-y-2.5 border-t border-border/60 pt-3.5"
              >
                <div class="flex items-center justify-between gap-2">
                  <h3 class="text-xs font-semibold">{{ t('jobUi.toolProviders') }}</h3>
                </div>
                <div class="grid gap-3 sm:grid-cols-2">
                  <div
                    v-for="[toolName, requiredTag] in capabilityEntries"
                    :key="toolName"
                    class="space-y-1.5 rounded-xl border border-border/70 bg-surface/60 p-3 text-xs"
                  >
                    <div class="flex flex-wrap items-center justify-between gap-1.5">
                      <div class="flex items-center gap-1.5">
                        <code class="rounded bg-muted px-1.5 py-0.5 font-semibold text-foreground">
                          {{ toolName }}
                        </code>
                        <Badge variant="outline">tag: {{ requiredTag }}</Badge>
                      </div>
                      <span
                        v-if="agentSpec.effective_tool_providers?.[toolName]"
                        class="font-mono text-[11px] text-muted-foreground"
                      >
                        {{ t('jobUi.effectiveToolProvider') }}:
                        <strong class="text-foreground">
                          {{ agentSpec.effective_tool_providers[toolName] }}
                        </strong>
                      </span>
                    </div>
                    <select
                      v-model="form.tool_providers[toolName]"
                      :data-tool-provider="toolName"
                      class="admin-input h-8 text-xs"
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
              </div>

              <p class="text-[11px] text-muted-foreground">
                {{ t('jobUi.toolProvidersHint') }}
              </p>
            </template>

            <template v-else>
              <div class="flex items-center justify-between gap-2">
                <div class="flex items-center gap-2">
                  <Workflow class="size-4 text-studio-violet" aria-hidden="true" />
                  <div>
                    <h2 id="agent-allowed-tools-heading" class="text-sm font-semibold">
                      {{ t('jobUi.rootDelegationTitle') }}
                    </h2>
                    <p class="mt-0.5 text-xs text-muted-foreground">
                      {{ t('jobUi.rootDelegationHint') }}
                    </p>
                  </div>
                </div>
                <Badge variant="secondary" class="tabular-nums">
                  {{ activeSubagents.length }}/{{ flowSubagents.length }}
                </Badge>
              </div>

              <ul class="divide-y divide-border/70 rounded-xl border border-border/70 bg-surface/40">
                <li
                  v-for="sub in flowSubagents"
                  :key="sub.id"
                  class="flex items-center justify-between gap-3 px-3.5 py-2.5"
                >
                  <div class="min-w-0 flex-1 space-y-0.5">
                    <div class="flex flex-wrap items-center gap-1.5">
                      <code class="rounded bg-muted px-1.5 py-0.5 text-xs font-semibold">
                        {{ sub.id }}
                      </code>
                      <Badge :variant="sub.enabled ? 'success' : 'outline'">
                        {{ sub.enabled ? t('jobUi.on') : t('jobUi.off') }}
                      </Badge>
                      <Badge variant="outline">model: {{ sub.effective_model }}</Badge>
                    </div>
                    <p class="truncate text-xs text-muted-foreground">{{ sub.description }}</p>
                  </div>
                  <Button
                    size="sm"
                    variant="outline"
                    class="h-7 px-2.5 text-xs"
                    @click="switchAgent(sub.id)"
                  >
                    {{ t('jobUi.configureAgent') }}
                    <ArrowRight class="size-3.5" aria-hidden="true" />
                  </Button>
                </li>
              </ul>
            </template>
          </section>

          <!-- Groupbox 5: Allowed Skills (Subagent) -->
          <section
            v-if="!isPrimary"
            id="agent-section-skills"
            class="studio-panel space-y-3.5 p-4"
            aria-labelledby="agent-allowed-skills-heading"
          >
            <div class="flex flex-wrap items-center justify-between gap-2">
              <div>
                <div class="flex items-center gap-2">
                  <h2 id="agent-allowed-skills-heading" class="text-sm font-semibold">
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

              <div class="flex flex-wrap items-center gap-1.5">
                <Button
                  size="sm"
                  variant="outline"
                  type="button"
                  class="h-7 px-2.5 text-xs"
                  :disabled="saving"
                  data-agent-create-skill
                  @click="openCreateSkillModal"
                >
                  <Plus class="size-3.5" aria-hidden="true" />
                  {{ t('jobUi.skillAdd') }}
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  type="button"
                  class="h-7 px-2.5 text-xs"
                  data-page="skills"
                  @click="router.push({ name: 'jobs-settings-skills' })"
                >
                  {{ t('jobUi.manageSkillLibrary') }}
                  <ArrowRight class="size-3.5" aria-hidden="true" />
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  type="button"
                  class="h-7 px-2 text-xs"
                  :disabled="
                    saving || form.allowed_skills.length === agentSpec.available_skills.length
                  "
                  @click="selectAllSkills"
                >
                  {{ t('jobUi.selectAll') }}
                </Button>
                <Button
                  v-if="
                    !isCustomSubagent &&
                    !sameStringArray(form.allowed_skills, agentSpec.default_allowed_skills ?? [])
                  "
                  size="sm"
                  variant="ghost"
                  type="button"
                  class="h-7 px-2 text-xs"
                  :disabled="saving"
                  @click="resetDefaultSkills"
                >
                  <RotateCcw class="size-3" aria-hidden="true" />
                  {{ t('jobUi.restoreDefault') }}
                </Button>
              </div>
            </div>

            <div class="flex flex-wrap gap-2">
              <div
                v-for="skill in agentSpec.available_skills"
                :key="skill"
                class="inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs transition-colors"
                :class="
                  form.allowed_skills.includes(skill)
                    ? 'border-studio-violet/40 bg-studio-violet/10 font-medium text-studio-violet'
                    : 'border-border/75 bg-surface/50 text-foreground hover:bg-accent/40'
                "
              >
                <label class="inline-flex cursor-pointer items-center gap-2">
                  <input
                    type="checkbox"
                    class="size-3.5 shrink-0 accent-primary"
                    :checked="form.allowed_skills.includes(skill)"
                    :disabled="saving"
                    @change="toggleSkill(skill, ($event.target as HTMLInputElement).checked)"
                  />
                  <code class="font-mono text-[11px]">{{ skill }}</code>
                </label>
                <button
                  type="button"
                  class="ml-1 inline-flex cursor-pointer items-center rounded px-1 py-0.5 text-[11px] text-muted-foreground hover:bg-accent hover:text-foreground"
                  :title="t('jobUi.skillViewEdit')"
                  :data-agent-edit-skill="skill"
                  @click="openEditSkillModal(skill)"
                >
                  <Eye class="size-3" aria-hidden="true" />
                </button>
              </div>
            </div>
          </section>
        </div>
      </div>
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
.prompt-markdown-preview :deep(h1) {
  font-size: 1.25rem;
}
.prompt-markdown-preview :deep(h2) {
  font-size: 1.1rem;
}
.prompt-markdown-preview :deep(h3) {
  font-size: 1rem;
}
.prompt-markdown-preview :deep(p) {
  margin: 0.5rem 0;
}
.prompt-markdown-preview :deep(ul),
.prompt-markdown-preview :deep(ol) {
  padding-left: 1.25rem;
  margin: 0.5rem 0;
}
.prompt-markdown-preview :deep(ul) {
  list-style: disc;
}
.prompt-markdown-preview :deep(ol) {
  list-style: decimal;
}
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

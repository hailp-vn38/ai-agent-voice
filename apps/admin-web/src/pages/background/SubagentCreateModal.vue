<script setup lang="ts">
import { BookOpen, Bot, Columns2, Eye, Pencil, Plus, Wrench, X } from '@lucide/vue'
import { computed, ref, watch } from 'vue'

import { agentJobsApi } from '@/api/agent-jobs'
import { formatApiError } from '@/api/errors'
import type {
  BackgroundProviderOption,
  BackgroundSettings,
  CreateSubagentInput,
  JobSkillSummary,
} from '@/api/types/agent-jobs'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { renderMarkdown } from '@/lib/markdown'
import {
  chatProviderOptions,
  domainToolsForCustomFlow,
  formatProviderOptionLabel,
  normalizeTools,
} from './backgroundAgentUtils'

const props = defineProps<{
  open: boolean
  initialFlowId: 'daily' | 'wiki'
  revision: number
  providers: BackgroundProviderOption[]
  skills: JobSkillSummary[]
  defaultToolsByFlow?: Record<'daily' | 'wiki', string[]>
}>()

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'created', updated: BackgroundSettings, createdId: string): void
}>()

const { t } = useI18n()

const saving = ref(false)
const error = ref('')
const editorMode = ref<'edit' | 'preview' | 'split'>('edit')

const form = ref<{
  id: string
  flow_id: 'daily' | 'wiki'
  enabled: boolean
  provider_ref: string | null
  steps: number
  allowed_tools: string[]
  allowed_skills: string[]
  description: string
  system_prompt: string
}>({
  id: '',
  flow_id: 'daily',
  enabled: true,
  provider_ref: null,
  steps: 15,
  allowed_tools: [],
  allowed_skills: [],
  description: '',
  system_prompt: '',
})

const SUBAGENT_ID_RE = /^[a-z0-9][a-z0-9_-]{0,63}$/
const RESERVED_IDS = new Set([
  'daily_root',
  'source_collector',
  'wiki_researcher',
  'report_stager',
  'media_creator',
  'media_processor',
  'reminder_planner',
  'wiki_root',
  'wiki_ingestor',
  'wiki_curator',
  'subagent',
  'skill',
  'default',
  'all',
])

const effectiveId = computed(() => form.value.id.trim())
const isValidId = computed(
  () => SUBAGENT_ID_RE.test(effectiveId.value) && !RESERVED_IDS.has(effectiveId.value),
)
const isValidFields = computed(() => {
  const desc = form.value.description.trim()
  const prompt = form.value.system_prompt.trim()
  const steps = Number(form.value.steps)
  if (!desc || desc.length > 1024) return false
  if (!prompt || prompt.length > 65536) return false
  if (!Number.isFinite(steps) || steps < 1 || steps > 100) return false
  return true
})
const canSave = computed(() => !saving.value && isValidId.value && isValidFields.value)

function toolsPoolForFlow(flowId: 'daily' | 'wiki'): string[] {
  const custom = props.defaultToolsByFlow?.[flowId]
  if (custom) return custom.filter(t => t !== 'skill')
  return domainToolsForCustomFlow(flowId)
}

const availableProviders = computed(() => chatProviderOptions(props.providers))
const domainTools = computed(() => toolsPoolForFlow(form.value.flow_id))
const promptLineCount = computed(() =>
  form.value.system_prompt ? form.value.system_prompt.split(/\r?\n/).length : 0,
)
const renderedPromptHtml = computed(() => renderMarkdown(form.value.system_prompt || ''))

watch(
  () => [props.open, props.initialFlowId] as const,
  ([open, initialFlowId]) => {
    if (!open) return
    error.value = ''
    editorMode.value = 'edit'
    form.value = {
      id: '',
      flow_id: initialFlowId,
      enabled: true,
      provider_ref: null,
      steps: 15,
      allowed_tools: [],
      allowed_skills: [],
      description: '',
      system_prompt: '',
    }
  },
  { immediate: true },
)

watch(
  () => form.value.flow_id,
  newFlow => {
    const allowedPool = new Set(toolsPoolForFlow(newFlow))
    form.value.allowed_tools = form.value.allowed_tools.filter(tool => allowedPool.has(tool))
  },
)

function toggleTool(tool: string, checked: boolean) {
  const set = new Set(form.value.allowed_tools)
  if (checked) set.add(tool)
  else set.delete(tool)
  form.value.allowed_tools = Array.from(set)
}

function selectAllTools() {
  form.value.allowed_tools = [...domainTools.value]
}

function clearAllTools() {
  form.value.allowed_tools = []
}

function toggleSkill(skillId: string, checked: boolean) {
  const set = new Set(form.value.allowed_skills)
  if (checked) set.add(skillId)
  else set.delete(skillId)
  form.value.allowed_skills = Array.from(set)
}

async function submit() {
  error.value = ''
  if (!isValidId.value) {
    error.value = t('jobUi.subagentValidationId')
    return
  }
  if (!isValidFields.value) {
    error.value = t('jobUi.subagentValidationFields')
    return
  }
  saving.value = true
  try {
    const skills = [...form.value.allowed_skills]
    const normalized = normalizeTools(form.value.allowed_tools, skills, false)
    const tools = normalized.length > 0 ? normalized : ['skill']
    const payload: CreateSubagentInput = {
      id: effectiveId.value,
      flow_id: form.value.flow_id,
      enabled: form.value.enabled,
      provider_ref: form.value.provider_ref || null,
      steps: Math.max(1, Math.min(100, Number(form.value.steps) || 15)),
      allowed_tools: tools,
      allowed_skills: skills,
      description: form.value.description.trim(),
      system_prompt: form.value.system_prompt.trim(),
    }
    const updated = await agentJobsApi.createSubagent(payload, props.revision)
    emit('created', updated, payload.id)
  } catch (err) {
    error.value = formatApiError(err)
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <div
    v-if="open"
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-xs"
    role="dialog"
    aria-modal="true"
    aria-labelledby="subagent-create-modal-title"
    data-subagent-create-modal
  >
    <div class="studio-panel flex max-h-[92vh] w-full max-w-4xl flex-col overflow-hidden shadow-2xl">
      <!-- Modal Header -->
      <div class="flex items-start justify-between gap-3 border-b border-border/70 px-6 py-4">
        <div class="min-w-0 space-y-1">
          <div class="flex flex-wrap items-center gap-2">
            <Bot class="size-4 text-studio-violet" aria-hidden="true" />
            <h2 id="subagent-create-modal-title" class="text-base font-semibold">
              {{ t('jobUi.subagentCreateTitle') }}
            </h2>
            <code
              v-if="effectiveId"
              class="rounded bg-muted px-2 py-0.5 text-xs font-semibold text-foreground"
            >
              data/orchestrator-prompts/{{ effectiveId }}.md
            </code>
            <Badge variant="outline">{{ t('jobUi.subagentCustomBadge') }}</Badge>
          </div>
          <p class="text-xs text-muted-foreground">
            {{ t('jobUi.subagentCreateSubtitle') }}
          </p>
        </div>
        <Button
          size="sm"
          variant="ghost"
          type="button"
          :aria-label="t('common.closeDialog')"
          data-subagent-modal-close
          @click="emit('close')"
        >
          <X class="size-4" aria-hidden="true" />
        </Button>
      </div>

      <!-- Modal Body -->
      <div class="flex-1 space-y-4 overflow-y-auto px-6 py-5 text-sm">
        <p
          v-if="error"
          class="rounded-xl border border-danger/30 bg-danger/10 px-4 py-3 text-xs text-danger"
          role="alert"
          data-subagent-modal-error
        >
          {{ error }}
        </p>

        <!-- Sub-Agent ID & Target Flow -->
        <div class="grid gap-4 sm:grid-cols-2">
          <label class="block space-y-1.5">
            <span class="text-xs font-medium">{{ t('jobUi.subagentIdLabel') }}</span>
            <input
              v-model="form.id"
              type="text"
              maxlength="64"
              class="admin-input font-mono text-xs"
              :placeholder="t('jobUi.subagentIdPlaceholder')"
              :disabled="saving"
              data-subagent-id-input
            />
            <span class="block text-[11px] text-muted-foreground">
              {{ t('jobUi.subagentIdHint') }}
            </span>
          </label>

          <label class="block space-y-1.5">
            <span class="text-xs font-medium">{{ t('jobUi.subagentFlowLabel') }}</span>
            <select
              v-model="form.flow_id"
              class="admin-input text-xs"
              :disabled="saving"
              data-subagent-flow-select
            >
              <option value="daily">{{ t('jobUi.flowDailyShort') }} (daily_root)</option>
              <option value="wiki">{{ t('jobUi.flowWikiShort') }} (wiki_root)</option>
            </select>
            <span class="block text-[11px] text-muted-foreground">
              {{ t('jobUi.subagentFlowHint') }}
            </span>
          </label>
        </div>

        <!-- LLM Provider & Steps -->
        <div class="grid gap-4 sm:grid-cols-2">
          <label class="block space-y-1.5">
            <span class="text-xs font-medium">{{ t('jobUi.providers') }}</span>
            <select
              v-model="form.provider_ref"
              class="admin-input text-xs"
              :disabled="saving"
              data-subagent-provider-select
            >
              <option :value="null">{{ t('jobUi.inheritProvider') }}</option>
              <option v-for="option in availableProviders" :key="option.ref" :value="option.ref">
                {{ formatProviderOptionLabel(option) }}
              </option>
            </select>
          </label>

          <label class="block space-y-1.5">
            <span class="text-xs font-medium">{{ t('jobUi.stepsLimit') }} (1 - 100)</span>
            <input
              v-model.number="form.steps"
              type="number"
              min="1"
              max="100"
              class="admin-input font-mono text-xs"
              :disabled="saving"
              data-subagent-steps-input
            />
          </label>
        </div>

        <!-- Role Description -->
        <label class="block space-y-1.5">
          <span class="text-xs font-medium">{{ t('jobUi.agentDescriptionLabel') }}</span>
          <input
            v-model="form.description"
            type="text"
            maxlength="1024"
            class="admin-input text-xs"
            :placeholder="t('jobUi.subagentDescriptionPlaceholder')"
            :disabled="saving"
            data-subagent-description-input
          />
        </label>

        <!-- Allowed Tools -->
        <div class="space-y-2 rounded-xl border border-border/70 bg-surface/40 p-3.5">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div class="flex items-center gap-2">
              <Wrench class="size-3.5 text-studio-violet" aria-hidden="true" />
              <span class="text-xs font-medium">{{ t('jobUi.toolsAllowed') }}</span>
              <Badge variant="secondary" class="tabular-nums">
                {{ form.allowed_tools.length }}/{{ domainTools.length }}
              </Badge>
            </div>
            <div class="flex items-center gap-1.5">
              <Button
                size="sm"
                variant="ghost"
                type="button"
                :disabled="saving"
                @click="selectAllTools"
              >
                {{ t('jobUi.selectAll') }}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                type="button"
                :disabled="saving"
                @click="clearAllTools"
              >
                {{ t('jobUi.clearAll') }}
              </Button>
            </div>
          </div>
          <div class="grid max-h-40 gap-1.5 overflow-y-auto sm:grid-cols-3">
            <label
              v-for="tool in domainTools"
              :key="tool"
              class="flex cursor-pointer items-center gap-2 rounded-lg border px-2.5 py-1.5 text-xs"
              :class="
                form.allowed_tools.includes(tool)
                  ? 'border-studio-violet/35 bg-studio-violet/5'
                  : 'border-border/60 bg-background/60 opacity-80'
              "
            >
              <input
                type="checkbox"
                class="size-3.5 accent-primary"
                :checked="form.allowed_tools.includes(tool)"
                :disabled="saving"
                :data-subagent-tool="tool"
                @change="toggleTool(tool, ($event.target as HTMLInputElement).checked)"
              />
              <code class="truncate text-[11px]">{{ tool }}</code>
            </label>
          </div>
        </div>

        <!-- Allowed Skills -->
        <div v-if="skills.length > 0" class="space-y-2 rounded-xl border border-border/70 bg-surface/40 p-3.5">
          <div class="flex items-center gap-2">
            <BookOpen class="size-3.5 text-studio-violet" aria-hidden="true" />
            <span class="text-xs font-medium">{{ t('jobUi.skillsAllowed') }}</span>
            <Badge variant="secondary" class="tabular-nums">
              {{ form.allowed_skills.length }}/{{ skills.length }}
            </Badge>
          </div>
          <div class="grid max-h-32 gap-1.5 overflow-y-auto sm:grid-cols-2">
            <label
              v-for="skill in skills"
              :key="skill.id"
              class="flex cursor-pointer items-center gap-2 rounded-lg border px-2.5 py-1.5 text-xs"
              :class="
                form.allowed_skills.includes(skill.id)
                  ? 'border-studio-violet/35 bg-studio-violet/5'
                  : 'border-border/60 bg-background/60 opacity-80'
              "
            >
              <input
                type="checkbox"
                class="size-3.5 accent-primary"
                :checked="form.allowed_skills.includes(skill.id)"
                :disabled="saving"
                :data-subagent-skill="skill.id"
                @change="toggleSkill(skill.id, ($event.target as HTMLInputElement).checked)"
              />
              <code class="truncate text-[11px] font-medium">{{ skill.id }}</code>
            </label>
          </div>
        </div>

        <!-- System Prompt Markdown Editor -->
        <div class="space-y-2">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <label for="subagent-prompt-textarea" class="text-xs font-medium">
              {{ t('jobUi.systemPromptLabel') }}
            </label>

            <div class="flex flex-wrap items-center gap-2">
              <span class="font-mono text-[11px] text-muted-foreground tabular-nums">
                {{ promptLineCount }} {{ t('jobUi.promptLines') }} · {{ form.system_prompt.length }} {{ t('jobUi.promptChars') }}
              </span>
              <div class="inline-flex rounded-lg border border-border bg-surface/80 p-0.5 text-xs">
                <button
                  type="button"
                  class="inline-flex items-center gap-1 rounded-md px-2.5 py-1 font-medium transition-colors"
                  :class="editorMode === 'edit' ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:text-foreground'"
                  @click="editorMode = 'edit'"
                >
                  <Pencil class="size-3" aria-hidden="true" />{{ t('jobUi.promptModeEdit') }}
                </button>
                <button
                  type="button"
                  class="inline-flex items-center gap-1 rounded-md px-2.5 py-1 font-medium transition-colors"
                  :class="editorMode === 'preview' ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:text-foreground'"
                  @click="editorMode = 'preview'"
                >
                  <Eye class="size-3" aria-hidden="true" />{{ t('jobUi.promptModePreview') }}
                </button>
                <button
                  type="button"
                  class="inline-flex items-center gap-1 rounded-md px-2.5 py-1 font-medium transition-colors"
                  :class="editorMode === 'split' ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:text-foreground'"
                  @click="editorMode = 'split'"
                >
                  <Columns2 class="size-3" aria-hidden="true" />{{ t('jobUi.promptModeSplit') }}
                </button>
              </div>
            </div>
          </div>

          <div
            class="grid gap-3"
            :class="editorMode === 'split' ? 'lg:grid-cols-2' : 'grid-cols-1'"
          >
            <textarea
              v-if="editorMode === 'edit' || editorMode === 'split'"
              id="subagent-prompt-textarea"
              v-model="form.system_prompt"
              rows="10"
              class="admin-input min-h-[220px] w-full resize-y font-mono text-xs leading-relaxed"
              :placeholder="t('jobUi.subagentPromptPlaceholder')"
              :disabled="saving"
              data-subagent-prompt-textarea
            />
            <div
              v-if="editorMode === 'preview' || editorMode === 'split'"
              class="min-h-[220px] max-h-[320px] overflow-y-auto rounded-xl border border-border/80 bg-surface/60 p-4 text-xs leading-relaxed"
              data-subagent-prompt-preview
              v-html="renderedPromptHtml"
            />
          </div>
        </div>
      </div>

      <!-- Modal Footer -->
      <div class="flex flex-wrap items-center justify-end gap-2 border-t border-border/70 px-6 py-4">
        <Button
          size="sm"
          variant="outline"
          type="button"
          :disabled="saving"
          @click="emit('close')"
        >
          {{ t('common.cancel') }}
        </Button>
        <Button
          size="sm"
          type="button"
          :disabled="!canSave"
          data-subagent-create-submit
          @click="submit"
        >
          <Plus class="size-3.5" aria-hidden="true" />
          {{ t('jobUi.subagentCreateSubmit') }}
        </Button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import {
  BookOpen,
  Bot,
  Columns2,
  Eye,
  FileText,
  Pencil,
  RefreshCw,
  RotateCcw,
  Save,
  Trash2,
} from '@lucide/vue'
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { agentJobsApi } from '@/api/agent-jobs'
import { formatApiError, isApiError } from '@/api/errors'
import type { BackgroundSettings, JobSkillDetail } from '@/api/types/agent-jobs'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useErrorToast, useSuccessToast } from '@/composables/useNotify'
import { renderMarkdown } from '@/lib/markdown'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()

const routeSkillId = computed(() => {
  const raw = route.params.skillId
  return typeof raw === 'string' ? raw : ''
})
const isCreateMode = computed(
  () => route.name === 'jobs-settings-skill-create' || !routeSkillId.value || routeSkillId.value === 'new',
)

const loading = ref(false)
const saving = ref(false)
const deleting = ref(false)
const error = ref('')
useErrorToast(() => error.value)
const requiresToolsError = ref('')
const notice = ref('')
useSuccessToast(() => notice.value, { onDismiss: () => (notice.value = '') })
const detail = ref<JobSkillDetail | null>(null)
const settings = ref<BackgroundSettings | null>(null)
const editorMode = ref<'edit' | 'preview' | 'split' | 'default'>('edit')

const form = ref({
  id: '',
  name: '',
  description: '',
  requiresTools: '',
  body: '',
})

let controller: AbortController | undefined

const SKILL_ID_RE = /^[a-z0-9][a-z0-9_-]{0,63}$/

const effectiveId = computed(() =>
  isCreateMode.value ? form.value.id.trim() : (routeSkillId.value || detail.value?.id || ''),
)

const isBuiltin = computed(() => Boolean(detail.value?.is_builtin))
const requiredByAgents = computed(() => {
  const skillId = detail.value?.id
  if (!skillId) return []
  return (settings.value?.agents ?? [])
    .filter(agent => agent.required_job_skills?.includes(skillId))
    .map(agent => agent.id)
})
const isValidId = computed(() => SKILL_ID_RE.test(effectiveId.value))

const isValidFields = computed(() => {
  const name = form.value.name.trim()
  const desc = form.value.description.trim()
  const body = form.value.body.trim()
  if (!name || name.length > 128 || /[\r\n"]/.test(name)) return false
  if (!desc || desc.length > 1024 || /[\r\n"]/.test(desc)) return false
  if (!body) return false
  return true
})

const canSave = computed(
  () => !loading.value && !saving.value && !deleting.value && isValidId.value && isValidFields.value,
)

const hasChangesFromDefault = computed(() => {
  const def = detail.value?.default
  if (!def) return false
  return (
    form.value.name.trim() !== def.name.trim() ||
    form.value.description.trim() !== def.description.trim() ||
    form.value.requiresTools !== (def.requires_tools ?? []).join(', ') ||
    form.value.body.trim() !== def.body.trim()
  )
})

const bodyLineCount = computed(() => (form.value.body ? form.value.body.split('\n').length : 0))
const bodyCharCount = computed(() => form.value.body.length)
const renderedBodyHtml = computed(() => renderMarkdown(form.value.body || ''))
const renderedDefaultBodyHtml = computed(() => renderMarkdown(detail.value?.default?.body || ''))

async function loadSkill(id: string) {
  controller?.abort()
  const pending = new AbortController()
  controller = pending
  loading.value = true
  error.value = ''
  requiresToolsError.value = ''
  notice.value = ''
  try {
    const loaded = await agentJobsApi.skill(id, pending.signal)
    if (pending.signal.aborted) return
    detail.value = loaded
    form.value = {
      id: loaded.id,
      name: loaded.name,
      description: loaded.description,
      requiresTools: (loaded.requires_tools ?? []).join(', '),
      body: loaded.body,
    }
    try {
      const currentSettings = await agentJobsApi.settings(pending.signal)
      if (pending.signal.aborted) return
      settings.value = currentSettings
    } catch {
      if (!pending.signal.aborted) settings.value = null
    }
  } catch (err) {
    if (pending.signal.aborted) return
    error.value = formatApiError(err)
  } finally {
    if (controller === pending) loading.value = false
  }
}

watch(
  () => [isCreateMode.value, routeSkillId.value] as const,
  ([createMode, skillId]) => {
    error.value = ''
    requiresToolsError.value = ''
    notice.value = ''
    editorMode.value = 'edit'
    if (createMode) {
      detail.value = null
      form.value = {
        id: '',
        name: '',
        description: '',
        requiresTools: '',
        body: '',
      }
    } else if (skillId) {
      void loadSkill(skillId)
    }
  },
  { immediate: true },
)

function restoreDefault() {
  const def = detail.value?.default
  if (!def) return
  form.value.name = def.name
  form.value.description = def.description
  form.value.requiresTools = (def.requires_tools ?? []).join(', ')
  form.value.body = def.body
}

async function save() {
  error.value = ''
  notice.value = ''
  requiresToolsError.value = ''
  if (!isValidId.value) {
    error.value = t('jobUi.skillValidationId')
    return
  }
  if (!isValidFields.value) {
    error.value = t('jobUi.skillValidationFields')
    return
  }
  saving.value = true
  try {
    const payload = {
      name: form.value.name.trim(),
      description: form.value.description.trim(),
      body: form.value.body,
      requires_tools: form.value.requiresTools
        .split(',')
        .map(tool => tool.trim())
        .filter(Boolean),
    }
    const saved = isCreateMode.value
      ? await agentJobsApi.createSkill(effectiveId.value, payload)
      : await agentJobsApi.updateSkill(effectiveId.value, payload, detail.value?.etag ?? '')
    detail.value = saved
    form.value = {
      id: saved.id,
      name: saved.name,
      description: saved.description,
      requiresTools: (saved.requires_tools ?? []).join(', '),
      body: saved.body,
    }
    notice.value = t('jobUi.skillSaved')
    if (isCreateMode.value) {
      await router.replace({ name: 'jobs-settings-skill', params: { skillId: saved.id } })
    }
  } catch (err) {
    if (isApiError(err) && err.code === 'invalid_skill_requires_tools') {
      requiresToolsError.value = t('jobUi.skillValidationRequiresTools')
    } else {
      error.value = formatApiError(err)
    }
  } finally {
    saving.value = false
  }
}

async function removeSkill() {
  if (isCreateMode.value || isBuiltin.value || !detail.value || deleting.value) return
  const id = detail.value.id
  if (typeof window !== 'undefined' && typeof window.confirm === 'function') {
    const confirmed = window.confirm(t('jobUi.skillDeleteConfirm', { id }))
    if (!confirmed) return
  }
  deleting.value = true
  error.value = ''
  notice.value = ''
  try {
    await agentJobsApi.deleteSkill(id, detail.value.etag)
    await router.push({ name: 'jobs-settings-skills' })
  } catch (err) {
    error.value = formatApiError(err)
  } finally {
    deleting.value = false
  }
}

onBeforeUnmount(() => controller?.abort())
</script>

<template>
  <section class="space-y-4">
    <DetailHeader
      :title="
        isCreateMode
          ? t('jobUi.skillCreateTitle')
          : form.name || detail?.name || effectiveId || t('jobUi.skillEditTitle')
      "
      :back-label="t('jobUi.skillsPageTitle')"
      @back="router.push({ name: 'jobs-settings-skills' })"
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
            {{ t('jobUi.skillDetailEyebrow') }}
          </span>
          <span aria-hidden="true">·</span>
          <span class="font-mono">SKILL.md · {{ effectiveId || 'new' }}</span>
        </div>
      </template>
      <template #actions>
        <template v-if="!isCreateMode && detail">
          <Badge :variant="isBuiltin ? 'secondary' : 'default'">
            {{ isBuiltin ? t('jobUi.skillBuiltinBadge') : t('jobUi.skillCustomBadge') }}
          </Badge>
          <Badge
            v-if="isBuiltin && detail.is_modified"
            variant="outline"
            class="border-warning/40 bg-warning/10 text-warning-foreground dark:text-warning"
          >
            {{ t('jobUi.customizedBadge') }}
          </Badge>
          <Badge
            v-if="isBuiltin && detail.default_has_update"
            variant="outline"
            class="border-studio-cyan/40 bg-studio-cyan/10 text-studio-cyan"
            data-skill-default-update
          >
            {{ t('jobUi.skillDefaultUpdateBadge') }}
          </Badge>
        </template>

        <Button
          v-if="!isCreateMode && isBuiltin && detail?.default && hasChangesFromDefault"
          size="sm"
          variant="outline"
          type="button"
          :disabled="saving || deleting"
          data-skill-restore-default
          @click="restoreDefault"
        >
          <RotateCcw class="size-3.5" aria-hidden="true" />
          {{ t('jobUi.skillRestoreDefault') }}
        </Button>

        <Button
          v-if="!isCreateMode && !isBuiltin && detail"
          size="sm"
          variant="outline"
          type="button"
          class="border-danger/40 text-danger hover:bg-danger/10"
          :disabled="saving || deleting"
          data-skill-delete
          @click="removeSkill"
        >
          <Trash2 class="size-3.5" aria-hidden="true" />
          {{ t('jobUi.skillDelete') }}
        </Button>

        <Button
          v-if="!isCreateMode && routeSkillId"
          size="sm"
          variant="outline"
          type="button"
          :disabled="loading || saving || deleting"
          @click="loadSkill(routeSkillId)"
        >
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" aria-hidden="true" />
          {{ t('jobUi.refresh') }}
        </Button>

        <Button
          size="sm"
          type="button"
          :disabled="!canSave"
          data-skill-save
          @click="save"
        >
          <Save class="size-4" aria-hidden="true" />
          {{ isCreateMode ? t('jobUi.skillCreateSubmit') : t('jobUi.save') }}
        </Button>
      </template>
    </DetailHeader>

    <div
      v-if="loading && !detail"
      class="studio-panel p-6 text-center text-sm text-muted-foreground"
      role="status"
    >
      {{ t('common.loading') }}
    </div>

    <!-- Expanded Layout: Compact Top Metadata & Usage + Full-Width SKILL.md Workspace -->
    <div v-else class="space-y-4">
      <!-- Top Groupbox: Metadata & Agent Usage -->
      <section class="studio-panel p-4">
        <div class="grid gap-4 lg:grid-cols-12">
          <!-- Left: ID, Name, Description -->
          <div class="space-y-3 lg:col-span-8">
            <div class="grid gap-3.5 sm:grid-cols-2">
              <label class="block space-y-1.5">
                <span class="block text-xs font-semibold">{{ t('jobUi.skillIdLabel') }}</span>
                <input
                  v-model="form.id"
                  type="text"
                  class="admin-input h-9 font-mono text-xs"
                  :placeholder="t('jobUi.skillIdPlaceholder')"
                  :readonly="!isCreateMode"
                  :disabled="!isCreateMode || saving || deleting"
                  data-skill-id-input
                />
                <span v-if="isCreateMode" class="block text-[11px] text-muted-foreground">
                  {{ t('jobUi.skillIdHint') }}
                </span>
              </label>

              <label class="block space-y-1.5">
                <span class="block text-xs font-semibold">{{ t('jobUi.skillNameLabel') }}</span>
                <input
                  v-model="form.name"
                  type="text"
                  maxlength="128"
                  class="admin-input h-9 text-xs"
                  :placeholder="t('jobUi.skillNamePlaceholder')"
                  :disabled="saving || deleting"
                  data-skill-name-input
                />
              </label>
            </div>

            <label class="block space-y-1.5">
              <span class="block text-xs font-semibold">{{ t('jobUi.skillDescriptionLabel') }}</span>
              <textarea
                v-model="form.description"
                rows="2"
                maxlength="1024"
                class="admin-textarea text-xs leading-relaxed"
                :placeholder="t('jobUi.skillDescriptionPlaceholder')"
                :disabled="saving || deleting"
                data-skill-description-input
              />
            </label>

            <div class="space-y-1.5 text-xs" data-skill-required-tools>
              <div class="font-semibold">{{ t('jobUi.skillRequiresToolsLabel') }}</div>
              <div v-if="(detail?.requires_tools ?? []).length" class="flex flex-wrap gap-1.5">
                <code
                  v-for="tool in detail?.requires_tools"
                  :key="tool"
                  class="rounded border border-border bg-muted px-2 py-0.5 font-mono text-[11px]"
                >{{ tool }}</code>
              </div>
              <span v-else class="text-muted-foreground">
                {{ t('jobUi.skillRequiresToolsEmpty') }}
              </span>
            </div>

            <label class="block space-y-1.5">
              <span class="block text-xs font-semibold">{{ t('jobUi.skillRequiresToolsEditLabel') }}</span>
              <input
                v-model="form.requiresTools"
                type="text"
                class="admin-input h-9 font-mono text-xs"
                :placeholder="t('jobUi.skillRequiresToolsPlaceholder')"
                :disabled="saving || deleting"
                data-skill-requires-tools
                @input="requiresToolsError = ''"
              />
              <span class="block text-[11px] text-muted-foreground">
                {{ t('jobUi.skillRequiresToolsHint') }}
              </span>
              <span
                v-if="requiresToolsError"
                class="block text-[11px] text-danger"
                role="alert"
                data-skill-requires-tools-error
              >
                {{ requiresToolsError }}
              </span>
            </label>
          </div>

          <!-- Right: Used By & Suggested For -->
          <div
            class="flex flex-col justify-between gap-3 border-t border-border/70 pt-3.5 lg:col-span-4 lg:border-t-0 lg:border-l lg:pl-4 lg:pt-0"
          >
            <div>
              <div class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                {{ t('jobUi.skillUsedBy') }}
              </div>
              <div class="flex flex-wrap gap-1.5">
                <template v-if="detail && detail.used_by_agents.length > 0">
                  <button
                    v-for="agId in detail.used_by_agents"
                    :key="agId"
                    type="button"
                    class="inline-flex cursor-pointer items-center gap-1.5 rounded-lg border border-border/80 bg-surface px-2.5 py-1.5 font-mono text-xs transition-colors hover:border-studio-violet/40 hover:bg-studio-violet/10 hover:text-studio-violet"
                    :data-agent="agId"
                    @click="router.push({ name: 'jobs-settings-agent', params: { agentId: agId } })"
                  >
                    <Bot class="size-3.5 text-studio-violet" aria-hidden="true" />
                    {{ agId }}
                  </button>
                </template>
                <small v-else class="text-xs text-muted-foreground">
                  {{ t('jobUi.skillUnused') }}
                </small>
              </div>
            </div>

            <div
              v-if="requiredByAgents.length > 0"
              class="border-t border-border/60 pt-3"
              data-skill-required-by
            >
              <div class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                {{ t('jobUi.skillRequiredBy') }}
              </div>
              <div class="flex flex-wrap gap-1.5">
                <button
                  v-for="agId in requiredByAgents"
                  :key="agId"
                  type="button"
                  class="inline-flex cursor-pointer items-center gap-1.5 rounded-lg border border-studio-cyan/40 bg-studio-cyan/10 px-2.5 py-1.5 font-mono text-xs text-studio-cyan transition-colors hover:bg-studio-cyan/15"
                  :data-skill-required-agent="agId"
                  @click="router.push({ name: 'jobs-settings-agent', params: { agentId: agId } })"
                >
                  <Bot class="size-3.5" aria-hidden="true" />
                  {{ agId }}
                </button>
              </div>
            </div>

            <div
              v-if="detail && detail.suggested_for_agents.length > 0"
              class="border-t border-border/60 pt-3"
            >
              <div class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                {{ t('jobUi.skillSuggestedFor') }}
              </div>
              <div class="flex flex-wrap gap-1.5">
                <Badge v-for="agId in detail.suggested_for_agents" :key="agId" variant="outline">
                  {{ agId }}
                </Badge>
              </div>
            </div>
          </div>
        </div>
      </section>

      <!-- Full-Width Groupbox: SKILL.md -->
      <section class="studio-panel space-y-3 p-4">
        <div class="flex flex-wrap items-center justify-between gap-3 border-b border-border/70 pb-3">
          <div class="flex flex-wrap items-center gap-2.5">
            <div class="flex items-center gap-2">
              <FileText class="size-4 text-studio-violet" aria-hidden="true" />
              <h2 class="text-sm font-semibold">SKILL.md</h2>
            </div>
            <span class="rounded-md border border-border/70 bg-surface px-2 py-0.5 font-mono text-xs text-muted-foreground">
              skills/{{ effectiveId || '...' }}/SKILL.md
            </span>
            <span class="hidden text-xs text-muted-foreground xl:inline">
              · {{ t('jobUi.skillDetailMarkdownHint') }}
            </span>
          </div>

          <div class="flex flex-wrap items-center gap-2.5">
            <span class="font-mono text-[11px] text-muted-foreground tabular-nums">
              {{ bodyLineCount }} {{ t('jobUi.promptLines') }} · {{ bodyCharCount }} {{ t('jobUi.promptChars') }}
            </span>

            <div
              class="inline-flex rounded-lg border border-border bg-surface/80 p-0.5 text-xs"
              role="tablist"
            >
              <button
                type="button"
                role="tab"
                :aria-selected="editorMode === 'edit'"
                data-skill-editor-mode="edit"
                class="inline-flex cursor-pointer items-center gap-1 rounded-md px-2.5 py-1 font-medium transition-colors"
                :class="
                  editorMode === 'edit'
                    ? 'bg-card text-studio-violet shadow-xs'
                    : 'text-muted-foreground hover:text-foreground'
                "
                @click="editorMode = 'edit'"
              >
                <Pencil class="size-3" aria-hidden="true" />
                {{ t('jobUi.promptModeEdit') }}
              </button>
              <button
                type="button"
                role="tab"
                :aria-selected="editorMode === 'preview'"
                data-skill-editor-mode="preview"
                class="inline-flex cursor-pointer items-center gap-1 rounded-md px-2.5 py-1 font-medium transition-colors"
                :class="
                  editorMode === 'preview'
                    ? 'bg-card text-studio-violet shadow-xs'
                    : 'text-muted-foreground hover:text-foreground'
                "
                @click="editorMode = 'preview'"
              >
                <Eye class="size-3" aria-hidden="true" />
                {{ t('jobUi.promptModePreview') }}
              </button>
              <button
                type="button"
                role="tab"
                :aria-selected="editorMode === 'split'"
                data-skill-editor-mode="split"
                class="inline-flex cursor-pointer items-center gap-1 rounded-md px-2.5 py-1 font-medium transition-colors"
                :class="
                  editorMode === 'split'
                    ? 'bg-card text-studio-violet shadow-xs'
                    : 'text-muted-foreground hover:text-foreground'
                "
                @click="editorMode = 'split'"
              >
                <Columns2 class="size-3" aria-hidden="true" />
                {{ t('jobUi.promptModeSplit') }}
              </button>
              <button
                v-if="!isCreateMode && isBuiltin && detail?.default"
                type="button"
                role="tab"
                :aria-selected="editorMode === 'default'"
                data-skill-editor-mode="default"
                class="inline-flex cursor-pointer items-center gap-1 rounded-md px-2.5 py-1 font-medium transition-colors"
                :class="
                  editorMode === 'default'
                    ? 'bg-card text-studio-violet shadow-xs'
                    : 'text-muted-foreground hover:text-foreground'
                "
                @click="editorMode = 'default'"
              >
                <BookOpen class="size-3" aria-hidden="true" />
                {{ t('jobUi.skillModeDefault') }}
              </button>
            </div>
          </div>
        </div>

        <div
          v-if="editorMode === 'default' && detail?.default"
          class="space-y-3 rounded-xl border border-studio-violet/30 bg-studio-violet/5 p-3.5 text-xs"
          data-skill-default-view
        >
          <div class="flex flex-wrap items-center justify-between gap-2 border-b border-border/60 pb-2">
            <span class="font-medium text-foreground">
              {{ t('jobUi.skillDefaultReadOnlyNotice', { id: effectiveId }) }}
            </span>
            <Button
              v-if="hasChangesFromDefault"
              size="sm"
              variant="outline"
              type="button"
              @click="restoreDefault(); editorMode = 'edit'"
            >
              <RotateCcw class="size-3.5" aria-hidden="true" />
              {{ t('jobUi.skillRestoreDefault') }}
            </Button>
          </div>
          <div
            class="skill-markdown-preview min-h-[540px] rounded-xl border border-border/80 bg-background p-4 text-xs leading-relaxed"
            v-html="renderedDefaultBodyHtml"
          />
        </div>

        <div
          v-else
          class="grid gap-3.5"
          :class="editorMode === 'split' ? 'lg:grid-cols-2' : 'grid-cols-1'"
        >
          <textarea
            v-if="editorMode === 'edit' || editorMode === 'split'"
            v-model="form.body"
            rows="26"
            spellcheck="false"
            class="admin-textarea h-[calc(100vh-340px)] min-h-[540px] w-full resize-y font-mono text-xs leading-relaxed"
            :placeholder="t('jobUi.skillBodyPlaceholder')"
            :disabled="saving || deleting"
            data-skill-body-textarea
          />
          <div
            v-if="editorMode === 'preview' || editorMode === 'split'"
            class="skill-markdown-preview min-h-[540px] rounded-xl border border-border/80 bg-surface/60 p-4 text-xs leading-relaxed"
            :class="editorMode === 'split' ? 'h-[calc(100vh-340px)] overflow-y-auto' : ''"
            data-skill-body-preview
            v-html="renderedBodyHtml"
          />
        </div>
      </section>
    </div>
  </section>
</template>

<style scoped>
.skill-markdown-preview :deep(h1),
.skill-markdown-preview :deep(h2),
.skill-markdown-preview :deep(h3),
.skill-markdown-preview :deep(h4) {
  font-weight: 650;
  margin-top: 1rem;
  margin-bottom: 0.45rem;
  color: var(--foreground);
}
.skill-markdown-preview :deep(h1) {
  font-size: 1.15rem;
}
.skill-markdown-preview :deep(h2) {
  font-size: 1.05rem;
}
.skill-markdown-preview :deep(h3) {
  font-size: 0.95rem;
}
.skill-markdown-preview :deep(p) {
  margin: 0.45rem 0;
}
.skill-markdown-preview :deep(ul),
.skill-markdown-preview :deep(ol) {
  padding-left: 1.25rem;
  margin: 0.45rem 0;
}
.skill-markdown-preview :deep(ul) {
  list-style: disc;
}
.skill-markdown-preview :deep(ol) {
  list-style: decimal;
}
.skill-markdown-preview :deep(code) {
  font-size: 0.75rem;
  padding: 0.1rem 0.35rem;
  border-radius: 0.25rem;
  background: var(--muted);
}
.skill-markdown-preview :deep(pre) {
  margin: 0.65rem 0;
  padding: 0.65rem;
  border-radius: 0.5rem;
  border: 1px solid var(--border);
  background: var(--card);
  overflow-x: auto;
}
.skill-markdown-preview :deep(pre code) {
  padding: 0;
  background: transparent;
}
</style>

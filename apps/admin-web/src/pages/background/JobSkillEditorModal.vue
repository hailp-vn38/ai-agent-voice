<script setup lang="ts">
import { BookOpen, Columns2, Eye, Pencil, RotateCcw, Save, Trash2, X } from '@lucide/vue'
import { computed, ref, watch } from 'vue'

import { agentJobsApi } from '@/api/agent-jobs'
import { formatApiError } from '@/api/errors'
import type { JobSkillDetail } from '@/api/types/agent-jobs'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { renderMarkdown } from '@/lib/markdown'

const props = defineProps<{
  open: boolean
  mode: 'create' | 'edit'
  skillId?: string | null
}>()

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'saved', detail: JobSkillDetail): void
  (e: 'deleted', skillId: string): void
}>()

const { t } = useI18n()

const loading = ref(false)
const saving = ref(false)
const deleting = ref(false)
const error = ref('')
const detail = ref<JobSkillDetail | null>(null)
const editorMode = ref<'edit' | 'preview' | 'split' | 'default'>('split')

const form = ref({
  id: '',
  name: '',
  description: '',
  body: '',
})

const SKILL_ID_RE = /^[a-z0-9][a-z0-9_-]{0,63}$/

const effectiveId = computed(() =>
  props.mode === 'create' ? form.value.id.trim() : (props.skillId ?? detail.value?.id ?? ''),
)

const isBuiltin = computed(() => Boolean(detail.value?.is_builtin))

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

const canSave = computed(() => !loading.value && !saving.value && !deleting.value && isValidId.value && isValidFields.value)

const hasChangesFromDefault = computed(() => {
  const def = detail.value?.default
  if (!def) return false
  return (
    form.value.name.trim() !== def.name.trim() ||
    form.value.description.trim() !== def.description.trim() ||
    form.value.body.trim() !== def.body.trim()
  )
})

const bodyLineCount = computed(() => {
  if (!form.value.body) return 0
  return form.value.body.split('\n').length
})

const bodyCharCount = computed(() => form.value.body.length)

const renderedBodyHtml = computed(() => renderMarkdown(form.value.body || ''))
const renderedDefaultBodyHtml = computed(() =>
  renderMarkdown(detail.value?.default?.body || ''),
)

async function loadSkill(id: string) {
  loading.value = true
  error.value = ''
  try {
    const loaded = await agentJobsApi.skill(id)
    detail.value = loaded
    form.value = {
      id: loaded.id,
      name: loaded.name,
      description: loaded.description,
      body: loaded.body,
    }
  } catch (err) {
    error.value = formatApiError(err)
  } finally {
    loading.value = false
  }
}

watch(
  () => [props.open, props.mode, props.skillId] as const,
  ([open, mode, skillId]) => {
    if (!open) return
    error.value = ''
    editorMode.value = mode === 'create' ? 'edit' : 'split'
    if (mode === 'create') {
      detail.value = null
      form.value = {
        id: '',
        name: '',
        description: '',
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
  form.value.body = def.body
}

async function save() {
  error.value = ''
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
    }
    const saved =
      props.mode === 'create'
        ? await agentJobsApi.createSkill(effectiveId.value, payload)
        : await agentJobsApi.updateSkill(effectiveId.value, payload, detail.value?.etag ?? '')
    detail.value = saved
    emit('saved', saved)
  } catch (err) {
    error.value = formatApiError(err)
  } finally {
    saving.value = false
  }
}

async function removeSkill() {
  if (props.mode !== 'edit' || isBuiltin.value || !detail.value) return
  const id = detail.value.id
  if (typeof window !== 'undefined' && typeof window.confirm === 'function') {
    const confirmed = window.confirm(t('jobUi.skillDeleteConfirm', { id }))
    if (!confirmed) return
  }
  deleting.value = true
  error.value = ''
  try {
    await agentJobsApi.deleteSkill(id, detail.value.etag)
    emit('deleted', id)
  } catch (err) {
    error.value = formatApiError(err)
  } finally {
    deleting.value = false
  }
}
</script>

<template>
  <div
    v-if="open"
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-xs"
    role="dialog"
    aria-modal="true"
    aria-labelledby="job-skill-modal-title"
    data-skill-editor-modal
  >
    <div class="studio-panel flex max-h-[92vh] w-full max-w-4xl flex-col overflow-hidden shadow-2xl">
      <!-- Modal Header -->
      <div class="flex items-start justify-between gap-3 border-b border-border/70 px-6 py-4">
        <div class="min-w-0 space-y-1">
          <div class="flex flex-wrap items-center gap-2">
            <BookOpen class="size-4 text-studio-violet" aria-hidden="true" />
            <h2 id="job-skill-modal-title" class="text-base font-semibold">
              {{ mode === 'create' ? t('jobUi.skillCreateTitle') : t('jobUi.skillEditTitle') }}
            </h2>
            <code
              v-if="effectiveId"
              class="rounded bg-muted px-2 py-0.5 text-xs font-semibold text-foreground"
            >
              skills/{{ effectiveId }}/SKILL.md
            </code>
            <Badge v-if="mode === 'edit'" :variant="isBuiltin ? 'secondary' : 'outline'">
              {{ isBuiltin ? t('jobUi.skillBuiltinBadge') : t('jobUi.skillCustomBadge') }}
            </Badge>
            <Badge v-if="mode === 'edit' && isBuiltin && detail?.is_modified" variant="outline">
              {{ t('jobUi.customizedBadge') }}
            </Badge>
          </div>
          <p class="text-xs text-muted-foreground">
            {{ t('jobUi.skillModalSubtitle') }}
          </p>
        </div>
        <Button
          size="sm"
          variant="ghost"
          type="button"
          :aria-label="t('common.closeDialog')"
          data-skill-modal-close
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
          data-skill-modal-error
        >
          {{ error }}
        </p>

        <div v-if="loading" class="py-10 text-center text-sm text-muted-foreground">
          {{ t('common.loading') }}
        </div>

        <template v-else>
          <!-- Agent Link Badges in Edit Mode -->
          <div
            v-if="mode === 'edit' && detail"
            class="flex flex-wrap items-center gap-3 rounded-xl border border-border/70 bg-surface/50 px-3.5 py-2.5 text-xs"
          >
            <div class="flex flex-wrap items-center gap-1.5">
              <span class="text-muted-foreground">{{ t('jobUi.skillUsedBy') }}:</span>
              <template v-if="detail.used_by_agents.length > 0">
                <Badge v-for="ag in detail.used_by_agents" :key="ag" variant="secondary">
                  {{ ag }}
                </Badge>
              </template>
              <span v-else class="text-muted-foreground">{{ t('jobUi.skillUnused') }}</span>
            </div>

            <div
              v-if="detail.suggested_for_agents.length > 0"
              class="flex flex-wrap items-center gap-1.5 border-l border-border/70 pl-3"
            >
              <span class="text-muted-foreground">{{ t('jobUi.skillSuggestedFor') }}:</span>
              <Badge v-for="ag in detail.suggested_for_agents" :key="ag" variant="outline">
                {{ ag }}
              </Badge>
            </div>
          </div>

          <!-- Skill ID + Display Name -->
          <div class="grid gap-4 sm:grid-cols-2">
            <label class="block space-y-1.5">
              <span class="text-xs font-medium">{{ t('jobUi.skillIdLabel') }}</span>
              <input
                v-model="form.id"
                type="text"
                class="admin-input font-mono text-xs"
                :placeholder="t('jobUi.skillIdPlaceholder')"
                :disabled="mode === 'edit' || saving || deleting"
                data-skill-id-input
              />
              <span class="block text-[11px] text-muted-foreground">
                {{ t('jobUi.skillIdHint') }}
              </span>
            </label>

            <label class="block space-y-1.5">
              <span class="text-xs font-medium">{{ t('jobUi.skillNameLabel') }}</span>
              <input
                v-model="form.name"
                type="text"
                maxlength="128"
                class="admin-input text-xs"
                :placeholder="t('jobUi.skillNamePlaceholder')"
                :disabled="saving || deleting"
                data-skill-name-input
              />
            </label>
          </div>

          <!-- Short Description -->
          <label class="block space-y-1.5">
            <span class="text-xs font-medium">{{ t('jobUi.skillDescriptionLabel') }}</span>
            <input
              v-model="form.description"
              type="text"
              maxlength="1024"
              class="admin-input text-xs"
              :placeholder="t('jobUi.skillDescriptionPlaceholder')"
              :disabled="saving || deleting"
              data-skill-description-input
            />
          </label>

          <!-- Markdown Body Editor with Edit / Preview / Split -->
          <div class="space-y-2">
            <div class="flex flex-wrap items-center justify-between gap-2">
              <label for="skill-body-textarea" class="text-xs font-medium">
                {{ t('jobUi.skillBodyLabel') }}
              </label>

              <div class="flex flex-wrap items-center gap-2">
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
                    class="inline-flex items-center gap-1 rounded-md px-2.5 py-1 font-medium transition-colors"
                    :class="editorMode === 'edit' ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:text-foreground'"
                    @click="editorMode = 'edit'"
                  >
                    <Pencil class="size-3" aria-hidden="true" />{{ t('jobUi.promptModeEdit') }}
                  </button>
                  <button
                    type="button"
                    role="tab"
                    :aria-selected="editorMode === 'preview'"
                    data-skill-editor-mode="preview"
                    class="inline-flex items-center gap-1 rounded-md px-2.5 py-1 font-medium transition-colors"
                    :class="editorMode === 'preview' ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:text-foreground'"
                    @click="editorMode = 'preview'"
                  >
                    <Eye class="size-3" aria-hidden="true" />{{ t('jobUi.promptModePreview') }}
                  </button>
                  <button
                    type="button"
                    role="tab"
                    :aria-selected="editorMode === 'split'"
                    data-skill-editor-mode="split"
                    class="inline-flex items-center gap-1 rounded-md px-2.5 py-1 font-medium transition-colors"
                    :class="editorMode === 'split' ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:text-foreground'"
                    @click="editorMode = 'split'"
                  >
                    <Columns2 class="size-3" aria-hidden="true" />{{ t('jobUi.promptModeSplit') }}
                  </button>
                  <button
                    v-if="mode === 'edit' && isBuiltin && detail?.default"
                    type="button"
                    role="tab"
                    :aria-selected="editorMode === 'default'"
                    data-skill-editor-mode="default"
                    class="inline-flex items-center gap-1 rounded-md px-2.5 py-1 font-medium transition-colors"
                    :class="editorMode === 'default' ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:text-foreground'"
                    @click="editorMode = 'default'"
                  >
                    <BookOpen class="size-3" aria-hidden="true" />{{ t('jobUi.skillModeDefault') }}
                  </button>
                </div>

                <Button
                  v-if="mode === 'edit' && isBuiltin && detail?.default && hasChangesFromDefault"
                  size="sm"
                  variant="outline"
                  type="button"
                  :disabled="saving || deleting"
                  data-skill-restore-default
                  @click="restoreDefault"
                >
                  <RotateCcw class="size-3.5" aria-hidden="true" />{{ t('jobUi.skillRestoreDefault') }}
                </Button>
              </div>
            </div>

            <div
              v-if="editorMode === 'default' && detail?.default"
              class="space-y-3 rounded-xl border border-studio-violet/30 bg-studio-violet/5 p-4 text-xs"
              data-skill-default-view
            >
              <div class="flex flex-wrap items-center justify-between gap-2 border-b border-border/60 pb-2.5">
                <span class="font-medium text-foreground">
                  {{ t('jobUi.skillDefaultReadOnlyNotice', { id: effectiveId }) }}
                </span>
                <Button
                  v-if="hasChangesFromDefault"
                  size="sm"
                  variant="outline"
                  type="button"
                  @click="restoreDefault(); editorMode = 'split'"
                >
                  <RotateCcw class="size-3.5" aria-hidden="true" />{{ t('jobUi.skillRestoreDefault') }}
                </Button>
              </div>
              <div class="grid gap-2 sm:grid-cols-2">
                <div>
                  <span class="text-muted-foreground">{{ t('jobUi.skillNameLabel') }}: </span>
                  <code class="font-semibold text-foreground">{{ detail.default.name }}</code>
                </div>
                <div>
                  <span class="text-muted-foreground">{{ t('jobUi.skillDescriptionLabel') }}: </span>
                  <span class="text-foreground">{{ detail.default.description }}</span>
                </div>
              </div>
              <div
                class="skill-markdown-preview max-h-[380px] overflow-y-auto rounded-xl border border-border/80 bg-background p-4 text-xs leading-relaxed"
                v-html="renderedDefaultBodyHtml"
              />
            </div>

            <div
              v-else
              class="grid gap-3"
              :class="editorMode === 'split' ? 'lg:grid-cols-2' : 'grid-cols-1'"
            >
              <textarea
                v-if="editorMode === 'edit' || editorMode === 'split'"
                id="skill-body-textarea"
                v-model="form.body"
                rows="14"
                class="admin-input min-h-[300px] w-full resize-y font-mono text-xs leading-relaxed"
                :placeholder="t('jobUi.skillBodyPlaceholder')"
                :disabled="saving || deleting"
                data-skill-body-textarea
              />
              <div
                v-if="editorMode === 'preview' || editorMode === 'split'"
                class="skill-markdown-preview min-h-[300px] max-h-[420px] overflow-y-auto rounded-xl border border-border/80 bg-surface/60 p-4 text-xs leading-relaxed"
                data-skill-body-preview
                v-html="renderedBodyHtml"
              />
            </div>
          </div>
        </template>
      </div>

      <!-- Modal Footer -->
      <div class="flex flex-wrap items-center justify-between gap-3 border-t border-border/70 px-6 py-4">
        <div>
          <Button
            v-if="mode === 'edit' && !isBuiltin && detail"
            size="sm"
            variant="outline"
            type="button"
            class="border-danger/40 text-danger hover:bg-danger/10"
            :disabled="saving || deleting"
            data-skill-delete
            @click="removeSkill"
          >
            <Trash2 class="size-3.5" aria-hidden="true" />{{ t('jobUi.skillDelete') }}
          </Button>
        </div>

        <div class="flex items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            type="button"
            :disabled="saving || deleting"
            @click="emit('close')"
          >
            {{ t('common.cancel') }}
          </Button>
          <Button
            size="sm"
            type="button"
            :disabled="!canSave"
            data-skill-save
            @click="save"
          >
            <Save class="size-3.5" aria-hidden="true" />
            {{ mode === 'create' ? t('jobUi.skillCreateSubmit') : t('jobUi.skillSaveSubmit') }}
          </Button>
        </div>
      </div>
    </div>
  </div>
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
.skill-markdown-preview :deep(h1) { font-size: 1.15rem; }
.skill-markdown-preview :deep(h2) { font-size: 1.05rem; }
.skill-markdown-preview :deep(h3) { font-size: 0.95rem; }
.skill-markdown-preview :deep(p) {
  margin: 0.45rem 0;
}
.skill-markdown-preview :deep(ul),
.skill-markdown-preview :deep(ol) {
  padding-left: 1.25rem;
  margin: 0.45rem 0;
}
.skill-markdown-preview :deep(ul) { list-style: disc; }
.skill-markdown-preview :deep(ol) { list-style: decimal; }
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

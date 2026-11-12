<script setup lang="ts">
import { ArrowLeft, ArrowRight, ArrowUpRight, Check, FileText, Globe2, Layers3, Link2, MessageSquareText, RefreshCw, TriangleAlert } from '@lucide/vue'
import { computed, reactive, ref, useId, watch } from 'vue'

import { formatApiError } from '@/api/errors'
import BaseModal from '@/components/admin/BaseModal.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import {
  providerTypes,
  templateCandidateProviders,
  templateLanguageOptions,
  type AgentTemplate,
  type ProviderInstance,
  type ProviderType,
} from '@/domain/admin'
import { providerTypeIcons } from '@/lib/providerTypeIcons'
import type { TemplateSetupInput } from '@/stores/admin'

const props = defineProps<{
  /** Omit to create a new global template. */
  template?: AgentTemplate
  providers: ProviderInstance[]
  /** Agents currently linking the template, used to show the blast radius on edit. */
  agentCount?: number
  /** Awaited by the form so it can own the submitting and error state. */
  save: (payload: TemplateSetupInput) => Promise<unknown>
}>()

const open = defineModel<boolean>({ required: true })

const { t, providerTypeLabel } = useI18n()

const LAST_STEP = 3
const languageListId = useId()
const promptId = useId()

const step = ref(1)
const submitting = ref(false)
const failure = ref<unknown>(null)

const form = reactive({
  name: '',
  description: '',
  language: templateLanguageOptions[0] as string,
  prompt: '',
  providerBindings: {} as Partial<Record<ProviderType, string>>,
})

const editing = computed(() => Boolean(props.template))

/**
 * The editor follows the catalog types offered by the current domain model.
 */
const bindableTypes = computed(() => providerTypes)

const steps = computed(() => [
  { label: t('templateStudio.identity'), hint: t('templateStudio.identityShort'), title: t('templateStudio.identityTitle'), description: t('templateStudio.identityHint'), icon: FileText },
  { label: t('templateStudio.content'), hint: t('templateStudio.contentShort'), title: t('templateStudio.contentTitle'), description: t('templateStudio.contentHint'), icon: MessageSquareText },
  { label: t('templateStudio.review'), hint: t('templateStudio.reviewShort'), title: t('templateStudio.reviewTitle'), description: t('templateForm.reviewIntro'), icon: Check },
])
const currentStep = computed(() => steps.value[step.value - 1]!)

function goToStep(target: number) {
  if (submitting.value || (target > 1 && !form.name.trim())) return
  failure.value = null
  step.value = target
}

const boundCount = computed(
  () => bindableTypes.value.filter((type) => Boolean(form.providerBindings[type])).length,
)

const canContinue = computed(() => step.value !== 1 || form.name.trim().length > 0)
const canSave = computed(() => form.name.trim().length > 0)

function candidatesFor(type: ProviderType) {
  return templateCandidateProviders(props.providers, type)
}

function boundProviderName(type: ProviderType) {
  const providerId = form.providerBindings[type]
  return providerId ? (props.providers.find((provider) => provider.id === providerId)?.name ?? providerId) : ''
}

function loadForm() {
  form.name = props.template?.name ?? ''
  form.description = props.template?.description ?? ''
  form.language = props.template?.language ?? (templateLanguageOptions[0] as string)
  form.prompt = props.template?.prompt ?? ''
  form.providerBindings = { ...(props.template?.providerBindings ?? {}) }
}

watch(
  () => [open.value, props.template] as const,
  () => {
    if (!open.value) return
    step.value = 1
    failure.value = null
    loadForm()
  },
  { immediate: true },
)

function next() {
  if (!submitting.value && canContinue.value && step.value < LAST_STEP) step.value += 1
}

function back() {
  if (submitting.value) return
  failure.value = null
  if (step.value > 1) step.value -= 1
}

async function submit() {
  if (!canSave.value || submitting.value) return
  submitting.value = true
  failure.value = null
  try {
    await props.save({
      id: props.template?.id,
      name: form.name.trim(),
      description: form.description.trim(),
      language: form.language.trim() || (templateLanguageOptions[0] as string),
      prompt: form.prompt,
      providerBindings: Object.fromEntries(
        bindableTypes.value
          .filter((type) => Boolean(form.providerBindings[type]))
          .map((type) => [type, form.providerBindings[type]]),
      ),
    })
    open.value = false
  } catch (cause) {
    // The step is kept so a rejected slot can be corrected without retyping.
    failure.value = cause
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <BaseModal
    v-model="open"
    :title="editing ? t('templateForm.editTitle') : t('templateForm.createTitle')"
    :description="t('templateStudio.subtitle')"
    :close-disabled="submitting"
    width-class="max-w-[1180px] h-[min(830px,90dvh)]"
    body-class="template-studio-body"
  >
    <div class="template-studio">
      <aside class="template-manuscript">
        <div class="manuscript-eyebrow"><span /> TEMPLATE STUDIO <span>{{ editing ? 'EDIT' : 'NEW' }}</span></div>
        <div class="manuscript-art" aria-hidden="true">
          <span class="manuscript-sheet sheet-back" /><span class="manuscript-sheet sheet-middle" />
          <span class="manuscript-sheet sheet-front"><MessageSquareText :size="21" :stroke-width="1.3" /><i /><i /><i /></span>
          <small>VOICE / BLUEPRINT</small>
        </div>
        <h3 class="manuscript-name">{{ form.name || t('templateStudio.untitled') }}</h3>
        <p class="manuscript-caption">{{ t('templateStudio.caption') }}</p>
        <div class="manuscript-meta">
          <span><Globe2 :size="12" aria-hidden="true" />{{ form.language || '—' }}</span>
          <span>{{ t('count.characters', { count: form.prompt.length }) }}</span>
        </div>
        <nav :aria-label="t('templateStudio.navigation')" class="template-steps">
          <button
            v-for="(item, index) in steps"
            :key="index"
            type="button"
            class="template-step"
            :class="{ 'is-visited': step > index + 1 }"
            :aria-current="step === index + 1 ? 'step' : undefined"
            :disabled="submitting || (index > 0 && !form.name.trim())"
            @click="goToStep(index + 1)"
          >
            <span class="step-marker"><Check v-if="step > index + 1" :size="12" aria-hidden="true" /><template v-else>0{{ index + 1 }}</template></span>
            <span class="step-copy"><strong>{{ item.label }}</strong><small>{{ item.hint }}</small></span>
          </button>
        </nav>
        <div class="manuscript-bindings">
          <p>{{ t('templateStudio.bindings') }}<strong>{{ boundCount }} / {{ bindableTypes.length }}</strong></p>
          <div class="binding-dots">
            <span v-for="type in bindableTypes" :key="type" :data-bound="Boolean(form.providerBindings[type])" :title="`${providerTypeLabel(type)}: ${boundProviderName(type) || t('templateForm.noProvider')}`"><component :is="providerTypeIcons[type]" :size="14" aria-hidden="true" /><span class="sr-only">{{ providerTypeLabel(type) }}: {{ boundProviderName(type) || t('templateForm.noProvider') }}</span></span>
          </div>
        </div>
      </aside>

      <div class="template-desk" :aria-busy="submitting">
        <header class="template-desk-header">
          <div><p class="template-desk-eyebrow">{{ t('templateStudio.chapter', { step }) }} / 03</p><h3>{{ currentStep.title }}</h3><p>{{ currentStep.description }}</p></div>
          <component :is="currentStep.icon" :size="24" :stroke-width="1.4" aria-hidden="true" />
        </header>
        <p v-if="template && agentCount" class="template-shared-notice"><TriangleAlert :size="15" aria-hidden="true" /><span>{{ t('templateForm.sharedWarning', { name: template.name, count: agentCount }) }}</span></p>
        <p v-if="failure" role="alert" class="template-failure"><TriangleAlert :size="15" aria-hidden="true" />{{ formatApiError(failure) }}</p>

        <div v-if="step === 1" class="template-identity">
          <div class="identity-fields">
            <label>
              <span class="field-label">{{ t('templateForm.name') }}</span>
              <input v-model="form.name" :disabled="submitting" class="admin-input" :placeholder="t('templateForm.namePlaceholder')" required />
            </label>
            <label>
              <span class="field-label">{{ t('templateForm.language') }}</span>
              <input v-model="form.language" :disabled="submitting" class="admin-input" :list="languageListId" :placeholder="t('templateForm.languagePlaceholder')" />
              <datalist :id="languageListId"><option v-for="option in templateLanguageOptions" :key="option" :value="option" /></datalist>
            </label>
          </div>
          <label>
            <span class="field-label">{{ t('templateForm.descriptionField') }}</span>
            <textarea v-model="form.description" :disabled="submitting" class="admin-textarea min-h-28" :placeholder="t('templateForm.descriptionPlaceholder')" />
          </label>
          <div class="template-identity-note"><Layers3 :size="18" aria-hidden="true" /><p>{{ t('templateForm.description') }}</p></div>
        </div>

        <div v-else-if="step === 2" class="template-compose">
          <div class="prompt-editor">
            <div class="prompt-editor-bar"><label :for="promptId"><MessageSquareText :size="13" aria-hidden="true" />{{ t('templateForm.prompt') }}</label><small>{{ t('count.characters', { count: form.prompt.length }) }}</small></div>
            <textarea :id="promptId" v-model="form.prompt" :disabled="submitting" spellcheck="false" :placeholder="t('templateForm.promptPlaceholder')" />
            <p class="prompt-editor-footer">{{ t('templateStudio.promptHint') }}</p>
          </div>
          <fieldset class="template-provider-fields" :disabled="submitting">
            <legend>{{ t('templateForm.providers') }}</legend>
            <p>{{ t('templateForm.providersHint') }}</p>
            <div class="template-binding-list">
              <label v-for="type in bindableTypes" :key="type" class="template-binding" :data-bound="Boolean(form.providerBindings[type])">
                <span class="binding-label"><component :is="providerTypeIcons[type]" :size="13" aria-hidden="true" />{{ providerTypeLabel(type) }}</span>
                <select v-model="form.providerBindings[type]" class="admin-input" :aria-label="providerTypeLabel(type)">
                  <option value="">{{ t('templateForm.noProvider') }}</option>
                  <option v-for="provider in candidatesFor(type)" :key="provider.id" :value="provider.id">{{ provider.name }} · {{ provider.adapter }}</option>
                </select>
              </label>
            </div>
          </fieldset>
        </div>

        <div v-else class="template-review">
          <dl class="template-review-meta">
            <div><dt>{{ t('templateForm.name') }}</dt><dd>{{ form.name }}</dd></div>
            <div><dt>{{ t('templateForm.reviewLanguage') }}</dt><dd>{{ form.language || templateLanguageOptions[0] }}</dd></div>
            <div v-if="form.description" class="review-description"><dt>{{ t('templateForm.descriptionField') }}</dt><dd>{{ form.description }}</dd></div>
          </dl>
          <div class="template-review-content">
            <div class="prompt-editor">
              <div class="prompt-editor-bar"><span><MessageSquareText :size="13" aria-hidden="true" />{{ t('templateForm.reviewPrompt') }}</span><small>{{ t('count.characters', { count: form.prompt.length }) }}</small></div>
              <pre v-if="form.prompt" class="review-prompt">{{ form.prompt }}</pre>
              <p v-else class="review-empty-prompt">{{ t('templateForm.reviewNoPrompt') }}</p>
            </div>
            <div class="review-bindings">
              <h4>{{ t('templateForm.providers') }} <span class="text-muted-foreground">{{ boundCount }}/{{ bindableTypes.length }}</span></h4>
              <ul><li v-for="type in bindableTypes" :key="type"><span><component :is="providerTypeIcons[type]" :size="13" aria-hidden="true" />{{ providerTypeLabel(type) }}</span><strong>{{ boundProviderName(type) || '—' }}</strong></li></ul>
            </div>
          </div>
          <p class="template-save-note"><Link2 :size="14" aria-hidden="true" />{{ t('templateForm.bindingNote') }}</p>
        </div>
      </div>
    </div>

    <template #footer>
      <div class="template-studio-footer">
        <div class="template-footer-progress"><span>0{{ step }} / 03</span><span class="template-progress-track" aria-hidden="true"><i v-for="index in LAST_STEP" :key="index" :class="{ 'is-filled': step >= index }" /></span></div>
        <div class="template-footer-actions">
          <Button v-if="step > 1" variant="ghost" :disabled="submitting" @click="back"><ArrowLeft class="size-3.5" aria-hidden="true" />{{ t('common.back') }}</Button>
          <Button v-else variant="ghost" :disabled="submitting" @click="open = false">{{ t('common.cancel') }}</Button>
          <Button v-if="step < LAST_STEP" class="template-primary-action" :disabled="submitting || !canContinue" @click="next">{{ t('common.continue') }}<ArrowRight class="size-3.5" aria-hidden="true" /></Button>
          <Button v-else class="template-primary-action" :disabled="submitting || !canSave" @click="submit"><RefreshCw v-if="submitting" class="size-3.5 animate-spin" aria-hidden="true" />{{ submitting ? t('templateForm.saving') : editing ? t('templateForm.save') : t('templateForm.create') }}<ArrowUpRight v-if="!submitting" class="size-3.5" aria-hidden="true" /></Button>
        </div>
      </div>
    </template>
  </BaseModal>
</template>

<style scoped src="./template-form-dialog.css"></style>

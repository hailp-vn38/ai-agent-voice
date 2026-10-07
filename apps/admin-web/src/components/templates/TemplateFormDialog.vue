<script setup lang="ts">
import { TriangleAlert } from '@lucide/vue'
import { computed, reactive, ref, watch } from 'vue'

import { formatApiError } from '@/api/errors'
import BaseModal from '@/components/admin/BaseModal.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import {
  providerTypes,
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
 * The API models four required core slots plus an optional `speaker` slot; `vision` is not a
 * Template slot. Every bindable type except `vision` is offered here.
 */
const bindableTypes = computed(() => providerTypes.filter((type) => type !== 'vision'))

const steps = computed(() => [
  t('templateForm.stepIdentity'),
  t('templateForm.stepContent'),
  t('templateForm.stepReview'),
])

const boundCount = computed(
  () => bindableTypes.value.filter((type) => Boolean(form.providerBindings[type])).length,
)

const canContinue = computed(() => step.value !== 1 || form.name.trim().length > 0)
const canSave = computed(() => form.name.trim().length > 0)

function candidatesFor(type: ProviderType) {
  return props.providers.filter((provider) => provider.type === type)
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
  if (canContinue.value && step.value < LAST_STEP) step.value += 1
}

function back() {
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
    :description="t('templateForm.description')"
    width-class="max-w-3xl"
  >
    <div class="space-y-5">
      <ol class="grid grid-cols-3 gap-2 text-center text-xs">
        <li
          v-for="(label, index) in steps"
          :key="label"
          :class="[
            'rounded-md border px-2 py-2',
            step === index + 1 ? 'border-primary bg-primary text-primary-foreground' : 'text-muted-foreground',
          ]"
        >
          {{ label }}
        </li>
      </ol>

      <p
        v-if="template && agentCount"
        class="flex gap-2.5 rounded-lg bg-muted/60 px-3 py-2.5 text-xs leading-relaxed text-muted-foreground"
      >
        <TriangleAlert class="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
        <span>{{ t('templateForm.sharedWarning', { name: template.name, count: agentCount }) }}</span>
      </p>

      <p
        v-if="failure"
        role="alert"
        class="rounded-lg border border-red-200 bg-red-50 px-3 py-2.5 text-sm text-red-700"
      >
        {{ formatApiError(failure) }}
      </p>

      <div v-if="step === 1" class="space-y-4">
        <div>
          <label class="block space-y-1.5">
            <span class="text-sm font-medium">{{ t('templateForm.name') }}</span>
            <input
              v-model="form.name"
              class="admin-input"
              :placeholder="t('templateForm.namePlaceholder')"
              required
            />
          </label>

        </div>

        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('templateForm.language') }}</span>
          <input
            v-model="form.language"
            class="admin-input"
            list="template-language-options"
            :placeholder="t('templateForm.languagePlaceholder')"
          />
          <datalist id="template-language-options">
            <option v-for="option in templateLanguageOptions" :key="option" :value="option" />
          </datalist>
        </label>

        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('templateForm.descriptionField') }}</span>
          <textarea
            v-model="form.description"
            class="admin-textarea min-h-20"
            :placeholder="t('templateForm.descriptionPlaceholder')"
          />
        </label>
      </div>

      <div v-else-if="step === 2" class="space-y-5">
        <label class="block space-y-1.5">
          <span class="text-sm font-medium">{{ t('templateForm.prompt') }}</span>
          <textarea
            v-model="form.prompt"
            class="admin-textarea min-h-44 font-mono text-xs"
            spellcheck="false"
            :placeholder="t('templateForm.promptPlaceholder')"
          />
        </label>

        <fieldset class="space-y-3">
          <legend class="text-sm font-medium">{{ t('templateForm.providers') }}</legend>
          <p class="text-xs text-muted-foreground">{{ t('templateForm.providersHint') }}</p>

          <div class="grid gap-3 sm:grid-cols-2">
            <label v-for="type in bindableTypes" :key="type" class="block space-y-1.5">
              <span class="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
                <component :is="providerTypeIcons[type]" class="size-3.5" aria-hidden="true" />
                {{ providerTypeLabel(type) }}
              </span>
              <select
                v-model="form.providerBindings[type]"
                class="admin-input"
                :aria-label="providerTypeLabel(type)"
              >
                <option value="">{{ t('templateForm.noProvider') }}</option>
                <option
                  v-for="provider in candidatesFor(type)"
                  :key="provider.id"
                  :value="provider.id"
                >
                  {{ provider.name }} · {{ provider.adapter }}
                </option>
              </select>
            </label>
          </div>
        </fieldset>
      </div>

      <div v-else class="space-y-4">
        <p class="text-sm text-muted-foreground">{{ t('templateForm.reviewIntro') }}</p>

        <dl class="grid gap-3 rounded-lg border p-4 text-sm sm:grid-cols-2">
          <div>
            <dt class="text-muted-foreground">{{ t('templateForm.name') }}</dt>
            <dd class="font-medium">{{ form.name }}</dd>
          </div>
          <div>
            <dt class="text-muted-foreground">{{ t('templateForm.reviewLanguage') }}</dt>
            <dd>{{ form.language }}</dd>
          </div>
          <div>
            <dt class="text-muted-foreground">{{ t('templateForm.providers') }}</dt>
            <dd>
              <Badge v-if="boundCount" variant="outline">{{ boundCount }}/{{ bindableTypes.length }}</Badge>
              <span v-else class="text-muted-foreground">{{ t('templateForm.noProvider') }}</span>
            </dd>
          </div>
          <div v-if="form.description" class="sm:col-span-2">
            <dt class="text-muted-foreground">{{ t('templateForm.descriptionField') }}</dt>
            <dd>{{ form.description }}</dd>
          </div>
        </dl>

        <div class="space-y-1.5">
          <p class="text-sm font-medium">{{ t('templateForm.reviewPrompt') }}</p>
          <pre
            v-if="form.prompt"
            class="max-h-40 overflow-auto rounded-lg bg-muted/60 p-3 font-mono text-xs whitespace-pre-wrap"
          >{{ form.prompt }}</pre>
          <p v-else class="text-sm text-muted-foreground">{{ t('templateForm.reviewNoPrompt') }}</p>
        </div>

        <div class="space-y-1.5">
          <p class="text-sm font-medium">{{ t('templateForm.providers') }}</p>
          <ul class="space-y-1 text-sm">
            <li
              v-for="type in bindableTypes"
              :key="type"
              class="flex items-center justify-between gap-3"
            >
              <span class="flex items-center gap-1.5 text-xs text-muted-foreground">
                <component :is="providerTypeIcons[type]" class="size-3.5" aria-hidden="true" />
                {{ providerTypeLabel(type) }}
              </span>
              <span class="truncate font-medium">{{ boundProviderName(type) || '—' }}</span>
            </li>
          </ul>
        </div>

        <p class="rounded-lg bg-amber-50 px-3 py-2.5 text-sm text-amber-900">
          {{ t('templateForm.bindingNote') }}
        </p>

        <details class="rounded-lg border p-3">
          <summary class="cursor-pointer text-sm font-medium">{{ t('templateForm.showJson') }}</summary>
          <pre class="mt-3 overflow-auto text-xs">{{
            JSON.stringify(
              {
                name: form.name.trim(),
                description: form.description.trim(),
                language: form.language.trim(),
                prompt: form.prompt,
                provider_bindings: form.providerBindings,
              },
              null,
              2,
            )
          }}</pre>
        </details>
      </div>
    </div>

    <template #footer>
      <div class="flex items-center justify-between gap-2">
        <Button v-if="step > 1" variant="outline" @click="back">
          {{ t('common.back') }}
        </Button>
        <Button v-else variant="outline" @click="open = false">
          {{ t('common.cancel') }}
        </Button>

        <Button v-if="step < LAST_STEP" :disabled="!canContinue" @click="next">
          {{ t('common.continue') }}
        </Button>
        <Button v-else :disabled="submitting || !canSave" @click="submit">
          {{ submitting ? t('templateForm.saving') : editing ? t('templateForm.save') : t('templateForm.create') }}
        </Button>
      </div>
    </template>
  </BaseModal>
</template>

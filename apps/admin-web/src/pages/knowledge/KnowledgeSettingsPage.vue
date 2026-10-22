<script setup lang="ts">
import { ArrowLeft, BookOpenText, Database, Network, Settings2 } from '@lucide/vue'
import { useRouter } from 'vue-router'
import { computed, onMounted, ref } from 'vue'
import { requestJson, jsonRequest } from '@/api/client'
import { formatApiError } from '@/api/errors'

import { useI18n } from '@/composables/useI18n'

const router = useRouter()
const { t } = useI18n()

import type { KnowledgeSettings as Settings, KnowledgeProvider } from '@/api/types/knowledge-processing'
const settings = ref<Settings | null>(null)
const native = computed(() => settings.value?.execution_version === 2)
function eligible(provider: KnowledgeProvider) {
  return native.value ? provider.tool_calling === true : provider.runtime_ready && provider.structured_qualified === true
}
function selectVersion(event: Event) {
  if (!settings.value) return
  const previous = settings.value
  const { enabled, analysis_provider_ref, trigger_mode, revision, runtime_ready, llm_providers } = previous
  const common = { enabled, analysis_provider_ref, trigger_mode, revision, runtime_ready, llm_providers }
  const pages = previous.execution_version === 2 ? previous.max_pages_per_source : previous.max_candidates_per_source
  settings.value = (event.target as HTMLSelectElement).value === '2'
    ? { ...common, execution_version: 2, max_pages_per_source: pages }
    : { ...common, execution_version: 1, publication_policy: 'review_all', max_candidates_per_source: pages }
}
const loading = ref(true)
const saving = ref(false)
const error = ref('')
const saved = ref(false)
const autoEligible = computed(() => settings.value?.llm_providers.some(p => p.reference === settings.value?.analysis_provider_ref && p.runtime_ready && p.structured_qualified) ?? false)
async function load() {
  loading.value = true
  error.value = ''
  try { settings.value = await requestJson<Settings>('/api/admin/knowledge/settings') }
  catch (cause) { error.value = formatApiError(cause) }
  finally { loading.value = false }
}
async function save() {
  if (!settings.value || saving.value) return
  const { enabled, analysis_provider_ref, trigger_mode, revision } = settings.value
  const versioned = settings.value.execution_version === 2
    ? { execution_version: 2, max_pages_per_source: settings.value.max_pages_per_source }
    : { ...(settings.value.execution_version ? { execution_version: 1 } : {}), publication_policy: settings.value.publication_policy, max_candidates_per_source: settings.value.max_candidates_per_source }
  saving.value = true
  error.value = ''
  saved.value = false
  try {
    settings.value = await requestJson<Settings>('/api/admin/knowledge/settings', jsonRequest('PUT', {
      enabled, analysis_provider_ref: analysis_provider_ref || null, trigger_mode, ...versioned,
    }), { revision })
    saved.value = true
  } catch (cause) { error.value = formatApiError(cause) }
  finally { saving.value = false }
}
onMounted(load)
const informationAreas = [
  { icon: Database, title: 'knowledgeV3.settings.vault', description: 'knowledgeV3.settings.vaultHint' },
  { icon: Network, title: 'knowledgeV3.settings.graph', description: 'knowledgeV3.settings.graphHint' },
  { icon: BookOpenText, title: 'knowledgeV3.settings.publishing', description: 'knowledgeV3.settings.publishingHint' },
] as const
</script>

<template>
  <section class="mx-auto max-w-5xl space-y-6" aria-label="Knowledge settings">
    <button type="button" class="inline-flex items-center gap-2 rounded-lg px-2 py-1 text-sm text-muted-foreground
      transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
      :aria-label="t('knowledgeV3.settings.back')" @click="router.push('/knowledge')">
      <ArrowLeft class="size-4" aria-hidden="true" />
      {{ t('knowledgeV3.settings.back') }}
    </button>

    <header class="space-y-3">
      <div class="flex size-11 items-center justify-center rounded-xl border bg-muted/50 text-foreground">
        <Settings2 class="size-5" aria-hidden="true" />
      </div>
      <h1 class="text-3xl font-semibold tracking-tight">{{ t('knowledgeV3.settings.title') }}</h1>
      <p class="max-w-2xl text-sm leading-6 text-muted-foreground">
        {{ t('knowledgeV3.settings.description') }}
      </p>
    </header>

    <p v-if="error" role="alert" class="rounded-lg border border-destructive/30 p-3 text-sm text-destructive">{{ error }}</p>
    <p v-if="loading" role="status" class="text-sm text-muted-foreground">{{ t('knowledgeV3.processing.loading') }}</p>
    <form v-if="settings" class="studio-panel space-y-5 p-5" @submit.prevent="save" @input="saved = false" @change="saved = false">
      <div class="flex items-center justify-between gap-4">
        <h2 class="text-base font-semibold">{{ t('knowledgeV3.processing.title') }}</h2>
        <span class="text-xs text-muted-foreground">{{ t(settings.runtime_ready ? 'knowledgeV3.processing.ready' : 'knowledgeV3.processing.unavailable') }}</span>
      </div>
      <label class="block space-y-2 text-sm">
        <span>{{ t('knowledgeV3.processing.execution') }}</span>
        <select name="execution_version" :value="settings.execution_version ?? 1" :disabled="saving" class="w-full rounded-lg border bg-background p-2" @change="selectVersion">
          <option value="1">{{ t('knowledgeV3.processing.legacy') }}</option>
          <option value="2">{{ t('knowledgeV3.processing.native') }}</option>
        </select>
      </label>
      <p v-if="settings.instruction_error" role="alert" class="text-sm text-destructive">{{ settings.instruction_error }}</p>
      <label class="flex items-center gap-3 text-sm">
        <input v-model="settings.enabled" name="enabled" type="checkbox" :disabled="saving" />
        {{ t('knowledgeV3.processing.enabled') }}
      </label>
      <div class="grid gap-5 md:grid-cols-2">
        <label class="space-y-2 text-sm">
          <span>{{ t('knowledgeV3.processing.provider') }}</span>
          <select v-model="settings.analysis_provider_ref" name="analysis_provider_ref" class="w-full rounded-lg border bg-background p-2" :disabled="saving">
            <option :value="null">{{ t('knowledgeV3.processing.chooseProvider') }}</option>
            <option v-for="provider in settings.llm_providers" :key="provider.reference" :value="provider.reference" :disabled="!eligible(provider)">
              {{ provider.name }} · {{ provider.availability ?? (provider.runtime_ready ? t('knowledgeV3.processing.ready') : t('knowledgeV3.processing.unavailable')) }}{{ !eligible(provider) ? ' · ' + t('knowledgeV3.processing.unqualified') : '' }}
            </option>
          </select>
        </label>
        <label v-if="settings.execution_version !== 2" class="space-y-2 text-sm">
          <span>{{ t('knowledgeV3.processing.policy') }}</span>
          <select v-model="settings.publication_policy" name="publication_policy" class="w-full rounded-lg border bg-background p-2" :disabled="saving">
            <option value="review_all">{{ t('knowledgeV3.processing.reviewAll') }}</option>
            <option value="draft_only">{{ t('knowledgeV3.processing.draftOnly') }}</option>
            <option value="auto_publish" :disabled="!autoEligible">{{ t('knowledgeV3.processing.autoPublish') }}</option>
          </select>
        </label>
        <label class="space-y-2 text-sm">
          <span>{{ t('knowledgeV3.processing.trigger') }}</span>
          <select v-model="settings.trigger_mode" name="trigger_mode" class="w-full rounded-lg border bg-background p-2" :disabled="saving">
            <option value="manual">{{ t('knowledgeV3.processing.manual') }}</option>
            <option value="auto_after_import">{{ t('knowledgeV3.processing.afterImport') }}</option>
          </select>
        </label>
        <p v-if="native" class="text-sm">{{ t('knowledgeV3.processing.fixedPublish') }}</p>
        <label v-if="settings.execution_version === 2" class="space-y-2 text-sm">
          <span>{{ t('knowledgeV3.processing.maxPages') }}</span>
          <input v-model.number="settings.max_pages_per_source" name="max_pages_per_source" type="number" min="1" max="5" required class="w-full rounded-lg border bg-background p-2" :disabled="saving" />
        </label>
        <label v-else class="space-y-2 text-sm">
          <span>{{ t('knowledgeV3.processing.maxCandidates') }}</span>
          <input v-model.number="settings.max_candidates_per_source" name="max_candidates_per_source" type="number" min="1" max="5" required class="w-full rounded-lg border bg-background p-2" :disabled="saving" />
        </label>
      </div>
      <p class="text-xs leading-5 text-muted-foreground">{{ t('knowledgeV3.processing.hint') }}</p>
      <div class="flex items-center gap-3">
        <button type="submit" :disabled="saving" class="rounded-lg bg-primary px-4 py-2 text-sm text-primary-foreground disabled:opacity-50">{{ t(saving ? 'knowledgeV3.processing.saving' : 'knowledgeV3.processing.save') }}</button>
        <button type="button" :disabled="saving" class="rounded-lg border px-4 py-2 text-sm" @click="load">{{ t('knowledgeV3.processing.reload') }}</button>
        <p v-if="saved" role="status" class="text-sm text-muted-foreground">{{ t('knowledgeV3.processing.saved') }}</p>
      </div>
    </form>

    <div class="grid gap-4 md:grid-cols-3">
      <article v-for="item in informationAreas" :key="item.title" class="studio-panel space-y-3 p-5">
        <div class="flex size-9 items-center justify-center rounded-lg bg-muted/70">
          <component :is="item.icon" class="size-[18px] text-muted-foreground" aria-hidden="true" />
        </div>
        <h2 class="text-sm font-semibold">{{ t(item.title) }}</h2>
        <p class="text-xs leading-5 text-muted-foreground">{{ t(item.description) }}</p>
      </article>
    </div>
  </section>
</template>

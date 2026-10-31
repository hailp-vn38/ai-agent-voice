<script setup lang="ts">
import { Bot, Cpu, ExternalLink, Info, RefreshCw, Save } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { agentJobsApi } from '@/api/agent-jobs'
import { formatApiError, isApiError } from '@/api/errors'
import type {
  BackgroundProviderOption,
  BackgroundSettings,
  BackgroundSettingsUpdate,
} from '@/api/types/agent-jobs'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import {
  chatProviderOptions as filterChatProviderOptions,
  findProvider as findCatalogProvider,
  formatProviderOptionLabel,
} from './backgroundAgentUtils'

const router = useRouter()
const { t } = useI18n()

const settings = ref<BackgroundSettings | null>(null)
const draft = ref<{
  orchestrator_llm_provider: string | null
  worker_llm_provider: string | null
}>({
  orchestrator_llm_provider: null,
  worker_llm_provider: null,
})
const loading = ref(true)
const saving = ref(false)
const error = ref('')
const notice = ref('')
const unavailable = ref(false)

let controller: AbortController | undefined

const providerOptions = computed(() => settings.value?.llm_providers ?? [])
const chatProviderOptions = computed(() => filterChatProviderOptions(providerOptions.value))

function findProvider(refId: string | null | undefined): BackgroundProviderOption | undefined {
  return findCatalogProvider(providerOptions.value, refId)
}

const dirty = computed(() => {
  if (!settings.value) return false
  return (
    settings.value.orchestrator_llm_provider !== draft.value.orchestrator_llm_provider ||
    settings.value.worker_llm_provider !== draft.value.worker_llm_provider
  )
})

const readyToSave = computed(() =>
  Boolean(
    settings.value &&
      dirty.value &&
      (!settings.value.enabled ||
        (draft.value.orchestrator_llm_provider && draft.value.worker_llm_provider)),
  ),
)

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
    draft.value = {
      orchestrator_llm_provider: config.orchestrator_llm_provider,
      worker_llm_provider: config.worker_llm_provider,
    }
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
      enabled: settings.value.enabled,
      orchestrator_llm_provider: draft.value.orchestrator_llm_provider,
      worker_llm_provider: draft.value.worker_llm_provider,
    }
    const result = await agentJobsApi.updateSettings(payload, settings.value.revision)
    settings.value = {
      ...result,
      llm_providers: result.llm_providers ?? settings.value.llm_providers,
      agents: result.agents ?? settings.value.agents,
      skills: result.skills ?? settings.value.skills,
    }
    draft.value = {
      orchestrator_llm_provider: settings.value.orchestrator_llm_provider,
      worker_llm_provider: settings.value.worker_llm_provider,
    }
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

onMounted(() => {
  void load()
})
onBeforeUnmount(() => controller?.abort())
</script>

<template>
  <section class="space-y-4">
    <DetailHeader
      :title="t('jobUi.providersPageTitle')"
      :back-label="t('jobUi.allSettingsBack')"
      @back="router.push({ name: 'jobs-settings' })"
    >
      <template #icon>
        <div
          class="grid size-9 shrink-0 place-items-center rounded-xl bg-warning/15 text-warning-foreground dark:text-warning"
        >
          <Cpu class="size-4.5" aria-hidden="true" />
        </div>
      </template>
      <template #details>
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span class="font-semibold uppercase tracking-wider text-studio-violet">
            {{ t('jobUi.providersPageEyebrow') }}
          </span>
          <span aria-hidden="true">·</span>
          <span>{{ t('jobUi.providersPageSubtitle') }}</span>
        </div>
      </template>
      <template #actions>
        <Button
          variant="outline"
          size="sm"
          :disabled="loading || saving"
          @click="router.push({ name: 'providers', query: { type: 'llm' } })"
        >
          <ExternalLink class="size-4" aria-hidden="true" />
          {{ t('jobUi.manageProviders') }}
        </Button>
        <Button variant="outline" size="sm" :disabled="loading || saving" @click="load">
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" aria-hidden="true" />
          {{ t('jobUi.refresh') }}
        </Button>
        <Button
          size="sm"
          :disabled="!readyToSave || saving"
          data-providers-save
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

    <template v-else-if="settings">
      <!-- Orchestrator · Root Agents -->
      <section class="studio-panel space-y-3 p-4">
        <div class="flex items-start justify-between gap-3">
          <div>
            <h2 class="text-sm font-semibold">{{ t('jobUi.providersOrchestratorTitle') }}</h2>
            <p class="mt-0.5 text-xs text-muted-foreground">
              {{ t('jobUi.providersOrchestratorSubtitle') }}
            </p>
          </div>
          <Bot class="size-5 shrink-0 text-studio-violet" aria-hidden="true" />
        </div>

        <label class="block space-y-1.5">
          <span class="block text-xs font-semibold">{{ t('jobUi.providersDefaultLabel') }}</span>
          <select
            v-model="draft.orchestrator_llm_provider"
            class="admin-input h-9 text-xs"
            data-input="primary"
            data-orchestrator-provider
            :disabled="saving"
          >
            <option :value="null">{{ t('jobUi.chooseProvider') }}</option>
            <option v-for="option in chatProviderOptions" :key="option.ref" :value="option.ref">
              {{ formatProviderOptionLabel(option) }}
            </option>
            <option
              v-if="
                draft.orchestrator_llm_provider &&
                !chatProviderOptions.some(p => p.ref === draft.orchestrator_llm_provider)
              "
              :value="draft.orchestrator_llm_provider"
            >
              {{ draft.orchestrator_llm_provider }} ({{ t('jobUi.unavailable') }})
            </option>
          </select>
        </label>

        <div class="flex flex-wrap items-center gap-1.5 text-xs">
          <template v-if="findProvider(draft.orchestrator_llm_provider)">
            <Badge v-if="findProvider(draft.orchestrator_llm_provider)?.model" variant="secondary">
              model: {{ findProvider(draft.orchestrator_llm_provider)?.model }}
            </Badge>
            <Badge
              v-for="tag in findProvider(draft.orchestrator_llm_provider)?.tags ?? ['chat', 'reasoning']"
              :key="tag"
              variant="outline"
            >
              {{ tag }}
            </Badge>
            <Badge
              v-if="findProvider(draft.orchestrator_llm_provider)?.context_limit_tokens"
              variant="outline"
            >
              Context {{ findProvider(draft.orchestrator_llm_provider)?.context_limit_tokens }}
            </Badge>
            <Badge v-if="findProvider(draft.orchestrator_llm_provider)?.max_tokens" variant="outline">
              max_tokens: {{ findProvider(draft.orchestrator_llm_provider)?.max_tokens }}
            </Badge>
            <Badge
              v-if="findProvider(draft.orchestrator_llm_provider)?.reasoning_effort"
              variant="outline"
            >
              reasoning: {{ findProvider(draft.orchestrator_llm_provider)?.reasoning_effort }}
            </Badge>
          </template>
          <template v-else>
            <Badge variant="outline">chat</Badge>
            <Badge variant="outline">reasoning</Badge>
          </template>
        </div>
      </section>

      <!-- Worker · Subagents -->
      <section class="studio-panel space-y-3 p-4">
        <div class="flex items-start justify-between gap-3">
          <div>
            <h2 class="text-sm font-semibold">{{ t('jobUi.providersWorkerTitle') }}</h2>
            <p class="mt-0.5 text-xs text-muted-foreground">
              {{ t('jobUi.providersWorkerSubtitle') }}
            </p>
          </div>
          <Cpu class="size-5 shrink-0 text-studio-violet" aria-hidden="true" />
        </div>

        <label class="block space-y-1.5">
          <span class="block text-xs font-semibold">{{ t('jobUi.providersDefaultLabel') }}</span>
          <select
            v-model="draft.worker_llm_provider"
            class="admin-input h-9 text-xs"
            data-input="worker"
            data-worker-provider
            :disabled="saving"
          >
            <option :value="null">{{ t('jobUi.chooseProvider') }}</option>
            <option v-for="option in chatProviderOptions" :key="option.ref" :value="option.ref">
              {{ formatProviderOptionLabel(option) }}
            </option>
            <option
              v-if="
                draft.worker_llm_provider &&
                !chatProviderOptions.some(p => p.ref === draft.worker_llm_provider)
              "
              :value="draft.worker_llm_provider"
            >
              {{ draft.worker_llm_provider }} ({{ t('jobUi.unavailable') }})
            </option>
          </select>
        </label>

        <div class="flex flex-wrap items-center gap-1.5 text-xs">
          <template v-if="findProvider(draft.worker_llm_provider)">
            <Badge v-if="findProvider(draft.worker_llm_provider)?.model" variant="secondary">
              model: {{ findProvider(draft.worker_llm_provider)?.model }}
            </Badge>
            <Badge
              v-for="tag in findProvider(draft.worker_llm_provider)?.tags ?? ['chat', 'reasoning']"
              :key="tag"
              variant="outline"
            >
              {{ tag }}
            </Badge>
            <Badge variant="outline">Provider inheritance</Badge>
            <Badge
              v-if="findProvider(draft.worker_llm_provider)?.context_limit_tokens"
              variant="outline"
            >
              Context {{ findProvider(draft.worker_llm_provider)?.context_limit_tokens }}
            </Badge>
            <Badge v-if="findProvider(draft.worker_llm_provider)?.max_tokens" variant="outline">
              max_tokens: {{ findProvider(draft.worker_llm_provider)?.max_tokens }}
            </Badge>
          </template>
          <template v-else>
            <Badge variant="outline">chat</Badge>
            <Badge variant="outline">reasoning</Badge>
            <Badge variant="outline">Provider inheritance</Badge>
          </template>
        </div>
      </section>

      <!-- Info Banner -->
      <div
        class="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-studio-violet/25 bg-studio-violet/8 px-3.5 py-2.5 text-xs text-foreground"
      >
        <div class="flex items-center gap-2">
          <Info class="size-4 shrink-0 text-studio-violet" aria-hidden="true" />
          <span>{{ t('jobUi.providersInfoBanner') }}</span>
        </div>
        <p
          v-if="settings.enabled && (!draft.orchestrator_llm_provider || !draft.worker_llm_provider)"
          class="font-medium text-warning"
        >
          {{ t('jobUi.providersRequired') }}
        </p>
      </div>
    </template>
  </section>
</template>

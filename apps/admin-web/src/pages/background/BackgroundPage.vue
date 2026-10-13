<script setup lang="ts">
import { Activity, Bot, CheckCircle2, Cpu, RefreshCw, Save, Settings2, ShieldCheck } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { agentJobsApi } from '@/api/agent-jobs'
import type { BackgroundSettings, BackgroundSettingsUpdate } from '@/api/types/agent-jobs'
import { formatApiError, isApiError } from '@/api/errors'
import PageHeader from '@/components/admin/PageHeader.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const { t } = useI18n()
const settings = ref<BackgroundSettings | null>(null)
const draft = ref<BackgroundSettingsUpdate>({
  enabled: false, orchestrator_llm_provider: null, worker_llm_provider: null,
})
const loading = ref(true)
const saving = ref(false)
const error = ref('')
const notice = ref('')
const unavailable = ref(false)
let controller: AbortController | undefined

const providerOptions = computed(() => settings.value?.llm_providers ?? [])
const dirty = computed(() => settings.value !== null && (
  settings.value.enabled !== draft.value.enabled ||
  settings.value.orchestrator_llm_provider !== draft.value.orchestrator_llm_provider ||
  settings.value.worker_llm_provider !== draft.value.worker_llm_provider
))
const readyToSave = computed(() => Boolean(
  settings.value && dirty.value && (!draft.value.enabled ||
  (draft.value.orchestrator_llm_provider && draft.value.worker_llm_provider)),
))
function restore(item: BackgroundSettings) {
  draft.value = {
    enabled: item.enabled,
    orchestrator_llm_provider: item.orchestrator_llm_provider,
    worker_llm_provider: item.worker_llm_provider,
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
    const result = await agentJobsApi.updateSettings(draft.value, settings.value.revision)
    settings.value = { ...result, llm_providers: result.llm_providers ?? settings.value.llm_providers }
    restore(result)
    notice.value = t('jobUi.saved')
  } catch (cause) {
    error.value = formatApiError(cause)
    if (isApiError(cause) && cause.status === 409) await load()
  } finally {
    saving.value = false
  }
}
onMounted(() => { void load() })
onBeforeUnmount(() => controller?.abort())
</script>

<template>
  <section class="space-y-6">
    <PageHeader :title="t('jobUi.background')" :description="t('jobUi.backgroundDescription')">
      <template #actions>
        <Button variant="outline" :disabled="loading || saving" @click="load">
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" />
          {{ t('jobUi.refresh') }}
        </Button>
        <Button :disabled="!readyToSave || loading || saving" @click="save">
          <Save class="size-4" />{{ t('jobUi.save') }}
        </Button>
      </template>
    </PageHeader>

    <div v-if="error" role="alert" class="rounded-lg border border-danger/30 bg-danger/5 p-4 text-sm text-danger">
      <p class="font-medium">{{ unavailable ? t('jobUi.backendPending') : t('jobUi.loadFailed') }}</p>
      <p class="mt-1 break-words">{{ error }}</p>
    </div>
    <p v-if="notice" role="status" class="rounded-lg border border-success/30 bg-success/5 p-3 text-sm text-success-foreground">{{ notice }}</p>
    <div v-if="loading" class="studio-panel p-8 text-center text-sm text-muted-foreground" role="status">{{ t('common.loading') }}</div>

    <template v-if="settings">
      <div class="grid gap-4 xl:grid-cols-[minmax(0,1.5fr)_minmax(260px,1fr)]">
        <div class="space-y-4">
          <section class="studio-panel p-5">
            <div class="flex items-start justify-between gap-4">
              <div class="flex items-start gap-3">
                <div class="flex size-10 shrink-0 items-center justify-center rounded-xl bg-studio-violet/10 text-studio-violet">
                  <Activity class="size-5" />
                </div>
                <div>
                  <h2 class="font-semibold">{{ t('jobUi.globalRuntime') }}</h2>
                  <p class="mt-1 text-sm text-muted-foreground">{{ t('jobUi.globalHint') }}</p>
                </div>
              </div>
              <label class="inline-flex cursor-pointer items-center gap-2 text-sm font-medium">
                <input v-model="draft.enabled" type="checkbox" class="size-4 accent-primary" :aria-label="t('jobUi.globalRuntime')" />
                {{ draft.enabled ? t('jobUi.on') : t('jobUi.off') }}
              </label>
            </div>
          </section>

          <section class="studio-panel space-y-5 p-5">
            <div class="flex items-center gap-2">
              <Settings2 class="size-4 text-studio-violet" />
              <h2 class="font-semibold">{{ t('jobUi.providers') }}</h2>
            </div>
            <div class="grid gap-4 sm:grid-cols-2">
              <label class="space-y-2">
                <span class="flex items-center gap-2 text-sm font-medium"><Bot class="size-4" />{{ t('jobUi.orchestrator') }}</span>
                <select v-model="draft.orchestrator_llm_provider" class="admin-input" :disabled="saving">
                  <option :value="null">{{ t('jobUi.chooseProvider') }}</option>
                  <option v-for="item in providerOptions" :key="item.ref" :value="item.ref">{{ item.name }} · {{ item.source }}</option>
                  <option v-if="draft.orchestrator_llm_provider && !providerOptions.some((p) => p.ref === draft.orchestrator_llm_provider)"
                    :value="draft.orchestrator_llm_provider">{{ draft.orchestrator_llm_provider }} ({{ t('jobUi.unavailable') }})</option>
                </select>
                <p class="text-xs text-muted-foreground">{{ t('jobUi.orchestratorHint') }}</p>
              </label>
              <label class="space-y-2">
                <span class="flex items-center gap-2 text-sm font-medium"><Cpu class="size-4" />{{ t('jobUi.worker') }}</span>
                <select v-model="draft.worker_llm_provider" class="admin-input" :disabled="saving">
                  <option :value="null">{{ t('jobUi.chooseProvider') }}</option>
                  <option v-for="item in providerOptions" :key="item.ref" :value="item.ref">{{ item.name }} · {{ item.source }}</option>
                  <option v-if="draft.worker_llm_provider && !providerOptions.some((p) => p.ref === draft.worker_llm_provider)"
                    :value="draft.worker_llm_provider">{{ draft.worker_llm_provider }} ({{ t('jobUi.unavailable') }})</option>
                </select>
                <p class="text-xs text-muted-foreground">{{ t('jobUi.workerHint') }}</p>
              </label>
            </div>
            <p v-if="draft.enabled && (!draft.orchestrator_llm_provider || !draft.worker_llm_provider)" class="text-xs text-warning">
              {{ t('jobUi.providersRequired') }}
            </p>
          </section>
        </div>

        <aside class="space-y-4">
          <section class="studio-panel p-5">
            <div class="flex items-center justify-between gap-2">
              <h2 class="text-sm font-semibold">{{ t('jobUi.availability') }}</h2>
              <Badge :variant="draft.enabled ? 'success' : 'secondary'">{{ draft.enabled ? t('jobUi.on') : t('jobUi.off') }}</Badge>
            </div>
            <div class="mt-4 flex items-start gap-3">
              <ShieldCheck class="mt-0.5 size-5 shrink-0 text-studio-cyan" />
              <p class="text-sm leading-relaxed text-muted-foreground">{{ t('jobUi.allAgents') }}</p>
            </div>
            <p class="mt-3 text-sm" :class="settings.runtime_ready ? 'text-success' : 'text-warning'">
              {{ settings.runtime_ready ? t('jobUi.runtimeReady') : t('jobUi.runtimeUnavailable') }}
            </p>
            <div class="mt-4 flex items-start gap-3">
              <CheckCircle2 class="mt-0.5 size-5 shrink-0 text-studio-violet" />
              <p class="text-sm leading-relaxed text-muted-foreground">{{ t('jobUi.noMcp') }}</p>
            </div>
          </section>
          <section class="studio-panel p-5">
            <h2 class="text-sm font-semibold">{{ t('jobUi.resourceLimits') }}</h2>
            <p class="mt-2 text-sm leading-relaxed text-muted-foreground">{{ t('jobUi.resourceHint') }}</p>
          </section>
        </aside>
      </div>
    </template>
  </section>
</template>

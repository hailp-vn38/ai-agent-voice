<script setup lang="ts">
import { Activity, ArrowRight, Bot, CheckCircle2, Cpu, RefreshCw, Save, Settings2, ShieldCheck, Workflow } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { agentJobsApi } from '@/api/agent-jobs'
import type { BackgroundSettings, BackgroundSettingsUpdate } from '@/api/types/agent-jobs'
import { formatApiError, isApiError } from '@/api/errors'
import PageHeader from '@/components/admin/PageHeader.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const router = useRouter()
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
const runningJobs = ref<number | null>(null)
const queuedJobs = ref<number | null>(null)
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
function restore(value: BackgroundSettings) {
  draft.value = {
    enabled: value.enabled,
    orchestrator_llm_provider: value.orchestrator_llm_provider,
    worker_llm_provider: value.worker_llm_provider,
  }
}
async function refreshCapacity(signal: AbortSignal) {
  try {
    const [running, queued] = await Promise.all([
      agentJobsApi.listJobs({ status: 'running' }, signal),
      agentJobsApi.listJobs({ status: 'queued' }, signal),
    ])
    if (signal.aborted) return
    runningJobs.value = running.total ?? null
    queuedJobs.value = queued.total ?? null
  } catch {
    if (!signal.aborted) { runningJobs.value = null; queuedJobs.value = null }
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
    void refreshCapacity(pending.signal)
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
    if (isApiError(cause) && cause.status === 409) {
      // Do not overwrite the user's unsaved form with a conflict response.
      settings.value = null
      await load()
    }
  } finally {
    saving.value = false
  }
}
onMounted(() => { void load() })
onBeforeUnmount(() => controller?.abort())
</script>

<template>
  <section class="space-y-5">
    <PageHeader :title="t('jobUi.background')" :description="t('jobUi.backgroundDescription')">
      <template #actions>
        <Button variant="outline" :disabled="loading || saving" @click="load">
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" />{{ t('jobUi.refresh') }}
        </Button>
      </template>
    </PageHeader>
    <div v-if="error" role="alert" class="rounded-lg border border-danger/30 bg-danger/5 p-4 text-sm text-danger">
      <p class="font-medium">{{ unavailable ? t('jobUi.backendPending') : t('jobUi.loadFailed') }}</p>
      <p class="mt-1 break-words">{{ error }}</p>
    </div>
    <p v-if="notice" role="status" class="rounded-lg border border-success/30 bg-success/5 p-3 text-sm text-success-foreground">{{ notice }}</p>

    <div class="rounded-xl border border-studio-violet/20 bg-studio-violet/5 p-4 text-sm text-foreground">
      <strong>{{ t('jobUi.globalScope') }}</strong> {{ t('jobUi.allAgents') }}
    </div>
    <div v-if="loading" class="studio-panel p-8 text-center text-sm text-muted-foreground" role="status">{{ t('common.loading') }}</div>

    <template v-if="settings && !loading">
      <section class="studio-panel p-5">
        <div class="flex flex-wrap items-center justify-between gap-5">
          <div class="flex items-start gap-3">
            <div class="grid size-10 shrink-0 place-items-center rounded-xl bg-studio-violet/10 text-studio-violet"><Activity class="size-5" /></div>
            <div>
              <h2 class="font-semibold">{{ t('jobUi.globalRuntime') }}</h2>
              <p class="mt-1 text-sm text-muted-foreground">{{ t('jobUi.globalHint') }}</p>
            </div>
          </div>
          <label class="flex cursor-pointer items-center gap-3 text-sm font-medium">
            <span>{{ draft.enabled ? t('jobUi.on') : t('jobUi.off') }}</span>
            <input v-model="draft.enabled" type="checkbox" role="switch" :aria-label="t('jobUi.globalRuntime')"
              class="size-5 accent-primary" :disabled="saving" />
          </label>
        </div>
      </section>

      <section class="studio-panel space-y-5 p-5">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <h2 class="flex items-center gap-2 font-semibold"><Settings2 class="size-5 text-studio-violet" />{{ t('jobUi.providerConfiguration') }}</h2>
          <span class="text-xs text-muted-foreground">{{ t('jobUi.deploymentWide') }}</span>
        </div>
        <div class="grid gap-4 md:grid-cols-2">
          <div class="min-w-0 rounded-xl border bg-background p-4">
            <div class="mb-4 flex items-center gap-3">
              <span class="grid size-8 shrink-0 place-items-center rounded-lg bg-studio-violet/10 font-semibold text-studio-violet">1</span>
              <div><h3 class="flex items-center gap-2 font-semibold"><Bot class="size-4" />{{ t('jobUi.orchestrator') }}</h3><p class="mt-1 text-xs text-muted-foreground">{{ t('jobUi.orchestratorHint') }}</p></div>
            </div>
            <label class="block space-y-2">
              <span class="text-xs font-medium text-muted-foreground">{{ t('jobUi.providers') }}</span>
              <select v-model="draft.orchestrator_llm_provider" class="admin-input" :disabled="saving">
                <option :value="null">{{ t('jobUi.chooseProvider') }}</option>
                <option v-for="option in providerOptions" :key="option.ref" :value="option.ref">{{ option.name }} · {{ option.source }}</option>
                <option v-if="draft.orchestrator_llm_provider && !providerOptions.some(p => p.ref === draft.orchestrator_llm_provider)"
                  :value="draft.orchestrator_llm_provider">{{ draft.orchestrator_llm_provider }} ({{ t('jobUi.unavailable') }})</option>
              </select>
            </label>
            <p class="mt-3 text-xs text-muted-foreground">{{ t('jobUi.providerCatalogHint') }}</p>
          </div>
          <div class="min-w-0 rounded-xl border bg-background p-4">
            <div class="mb-4 flex items-center gap-3">
              <span class="grid size-8 shrink-0 place-items-center rounded-lg bg-studio-violet/10 font-semibold text-studio-violet">2</span>
              <div><h3 class="flex items-center gap-2 font-semibold"><Cpu class="size-4" />{{ t('jobUi.worker') }}</h3><p class="mt-1 text-xs text-muted-foreground">{{ t('jobUi.workerHint') }}</p></div>
            </div>
            <label class="block space-y-2">
              <span class="text-xs font-medium text-muted-foreground">{{ t('jobUi.providers') }}</span>
              <select v-model="draft.worker_llm_provider" class="admin-input" :disabled="saving">
                <option :value="null">{{ t('jobUi.chooseProvider') }}</option>
                <option v-for="option in providerOptions" :key="option.ref" :value="option.ref">{{ option.name }} · {{ option.source }}</option>
                <option v-if="draft.worker_llm_provider && !providerOptions.some(p => p.ref === draft.worker_llm_provider)"
                  :value="draft.worker_llm_provider">{{ draft.worker_llm_provider }} ({{ t('jobUi.unavailable') }})</option>
              </select>
            </label>
            <p class="mt-3 text-xs text-muted-foreground">{{ t('jobUi.providerCatalogHint') }}</p>
          </div>
        </div>
        <div class="flex flex-wrap items-center justify-between gap-3 border-t pt-4">
          <div>
            <p class="text-xs text-muted-foreground">{{ t('jobUi.sharedProviderHint') }}</p>
            <p v-if="draft.enabled && (!draft.orchestrator_llm_provider || !draft.worker_llm_provider)" class="mt-1 text-xs text-warning">
              {{ t('jobUi.providersRequired') }}
            </p>
          </div>
          <Button :disabled="!readyToSave || saving" @click="save"><Save class="size-4" />{{ t('jobUi.save') }}</Button>
        </div>
      </section>

      <div class="grid gap-4 xl:grid-cols-[minmax(0,1.35fr)_minmax(280px,1fr)]">
        <section class="studio-panel p-5">
          <h2 class="mb-5 flex items-center gap-2 font-semibold"><Workflow class="size-4 text-studio-violet" />{{ t('jobUi.executionFlow') }}</h2>
          <div class="flex flex-wrap items-center gap-2 text-xs">
            <span class="rounded-lg border bg-studio-violet/10 px-3 py-2 text-studio-violet">{{ t('jobUi.orchestrator') }}</span>
            <ArrowRight class="size-4 text-muted-foreground" />
            <span class="rounded-lg border bg-studio-violet/10 px-3 py-2 text-studio-violet">{{ t('jobUi.worker') }}</span>
            <ArrowRight class="size-4 text-muted-foreground" />
            <span class="rounded-lg border bg-success/10 px-3 py-2 text-success">{{ t('jobUi.pythonPlugin') }}</span>
            <ArrowRight class="size-4 text-muted-foreground" />
            <span class="rounded-lg border bg-muted px-3 py-2">{{ t('jobUi.artifacts') }}</span>
          </div>
          <div class="mt-5 flex items-start gap-2 text-sm text-muted-foreground"><CheckCircle2 class="mt-0.5 size-4 shrink-0 text-studio-cyan" /><p>{{ t('jobUi.noMcp') }}</p></div>
          <Button class="mt-4" variant="outline" @click="router.push({ name: 'jobs' })">{{ t('jobUi.jobs') }}<ArrowRight class="size-4" /></Button>
        </section>
        <section class="studio-panel p-5">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <h2 class="font-semibold">{{ t('jobUi.runtimeCapacity') }}</h2>
            <Badge :variant="settings.runtime_ready ? 'success' : 'secondary'">{{ settings.runtime_ready ? t('jobUi.runtimeReady') : t('jobUi.runtimeUnavailable') }}</Badge>
          </div>
          <div class="mt-4 divide-y text-sm">
            <div class="flex items-center justify-between gap-2 py-3"><span class="text-muted-foreground">{{ t('jobUi.runningJobs') }}</span><strong>{{ runningJobs ?? '—' }}</strong></div>
            <div class="flex items-center justify-between gap-2 py-3"><span class="text-muted-foreground">{{ t('jobUi.queuedJobs') }}</span><strong>{{ queuedJobs ?? '—' }}</strong></div>
            <div class="flex items-center justify-between gap-2 py-3"><span class="text-muted-foreground">{{ t('jobUi.workerConcurrency') }}</span><span class="text-muted-foreground">{{ t('jobUi.serverConfigured') }}</span></div>
            <div class="flex items-center justify-between gap-2 py-3"><span class="text-muted-foreground">{{ t('jobUi.voiceReserved') }}</span><span class="inline-flex items-center gap-2"><ShieldCheck class="size-4 text-studio-cyan" />{{ t('jobUi.serverConfigured') }}</span></div>
          </div>
          <p class="mt-4 text-xs text-muted-foreground">{{ t('jobUi.capacityHint') }}</p>
        </section>
      </div>
    </template>
  </section>
</template>

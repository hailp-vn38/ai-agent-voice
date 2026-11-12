<script setup lang="ts">
import { Activity, Cpu, RefreshCw, Save } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { agentJobsApi } from '@/api/agent-jobs'
import { formatApiError, isApiError } from '@/api/errors'
import type { BackgroundSettings, BackgroundSettingsUpdate } from '@/api/types/agent-jobs'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useErrorToast, useSuccessToast } from '@/composables/useNotify'

const router = useRouter()
const { t } = useI18n()

const settings = ref<BackgroundSettings | null>(null)
const draftEnabled = ref(false)
const loading = ref(true)
const saving = ref(false)
const error = ref('')
useErrorToast(() => error.value, { title: () => (unavailable.value ? t('jobUi.backendPending') : t('jobUi.loadFailed')), retry: () => load() })
const notice = ref('')
useSuccessToast(() => notice.value, { onDismiss: () => (notice.value = '') })
const unavailable = ref(false)

let controller: AbortController | undefined

const agentsCatalog = computed(() => settings.value?.agents ?? [])
const rootAgentsCount = computed(
  () => agentsCatalog.value.filter(a => a.mode === 'primary').length,
)
const activeSubagentsCount = computed(
  () => agentsCatalog.value.filter(a => a.mode !== 'primary' && a.enabled).length,
)

const dirty = computed(() => {
  if (!settings.value) return false
  return settings.value.enabled !== draftEnabled.value
})

const missingProviders = computed(() => {
  if (!settings.value) return false
  return Boolean(
    draftEnabled.value &&
      (!settings.value.orchestrator_llm_provider || !settings.value.worker_llm_provider),
  )
})

const readyToSave = computed(() => Boolean(settings.value && dirty.value && !missingProviders.value))

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
    draftEnabled.value = config.enabled
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
      enabled: draftEnabled.value,
      orchestrator_llm_provider: settings.value.orchestrator_llm_provider,
      worker_llm_provider: settings.value.worker_llm_provider,
    }
    const result = await agentJobsApi.updateSettings(payload, settings.value.revision)
    settings.value = {
      ...result,
      llm_providers: result.llm_providers ?? settings.value.llm_providers,
      agents: result.agents ?? settings.value.agents,
      skills: result.skills ?? settings.value.skills,
    }
    draftEnabled.value = settings.value.enabled
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
    <DetailHeader theme="jobs"
      :title="t('jobUi.runtimePageTitle')"
      :back-label="t('jobUi.allSettingsBack')"
      @back="router.push({ name: 'jobs-settings' })"
    >
      <template #icon>
        <div class="grid size-9 shrink-0 place-items-center rounded-xl bg-success/12 text-success">
          <Activity class="size-4.5" aria-hidden="true" />
        </div>
      </template>
      <template #details>
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span class="font-semibold uppercase tracking-wider text-studio-violet">
            {{ t('jobUi.runtimePageEyebrow') }}
          </span>
          <span aria-hidden="true">·</span>
          <span>{{ t('jobUi.runtimePageSubtitle') }}</span>
        </div>
      </template>
      <template #actions>
        <Button variant="outline" size="sm" :disabled="loading || saving" @click="load">
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" aria-hidden="true" />
          {{ t('jobUi.refresh') }}
        </Button>
        <Button
          size="sm"
          :disabled="!readyToSave || saving"
          data-runtime-save
          @click="save"
        >
          <Save class="size-4" aria-hidden="true" />
          {{ t('jobUi.save') }}
        </Button>
      </template>
    </DetailHeader>

    <div
      v-if="loading && !settings"
      class="studio-panel p-6 text-center text-sm text-muted-foreground"
      role="status"
    >
      {{ t('common.loading') }}
    </div>

    <template v-else-if="settings">
      <!-- Global Runtime Switch Groupbox -->
      <section class="studio-panel p-4">
        <div class="flex flex-wrap items-center justify-between gap-4">
          <div class="space-y-1">
            <div class="flex flex-wrap items-center gap-2">
              <Activity class="size-4 text-studio-violet" aria-hidden="true" />
              <h2 class="text-sm font-semibold">{{ t('jobUi.globalRuntime') }}</h2>
              <Badge :variant="draftEnabled ? 'success' : 'outline'">
                {{ draftEnabled ? t('jobUi.on') : t('jobUi.off') }}
              </Badge>
            </div>
            <p class="text-xs text-muted-foreground">
              {{ t('jobUi.globalHint') }}
            </p>
          </div>

          <label class="flex cursor-pointer items-center gap-2.5 text-xs font-medium">
            <span>{{ draftEnabled ? t('jobUi.on') : t('jobUi.off') }}</span>
            <span class="relative inline-flex shrink-0 items-center">
              <input
                v-model="draftEnabled"
                type="checkbox"
                role="switch"
                :aria-label="t('jobUi.globalRuntime')"
                data-toggle="runtime"
                class="peer sr-only"
                :disabled="saving"
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

        <div
          v-if="missingProviders"
          class="mt-3 flex flex-wrap items-center justify-between gap-3 rounded-xl border border-warning/35 bg-warning/10 px-3.5 py-2.5 text-xs text-warning-foreground dark:text-warning"
        >
          <span>{{ t('jobUi.providersRequired') }}</span>
          <Button
            size="sm"
            variant="outline"
            type="button"
            @click="router.push({ name: 'jobs-settings-providers' })"
          >
            <Cpu class="size-3.5" aria-hidden="true" />
            {{ t('jobUi.providersPageTitle') }}
          </Button>
        </div>
      </section>

      <!-- Read-only System Status Groupbox -->
      <section class="studio-panel p-4">
        <div class="mb-3 flex flex-wrap items-center justify-between gap-2">
          <div>
            <h2 class="text-sm font-semibold">{{ t('jobUi.runtimeStatusBoxTitle') }}</h2>
            <p class="mt-0.5 text-xs text-muted-foreground">
              {{ t('jobUi.runtimeStatusBoxSubtitle') }}
            </p>
          </div>
          <Badge variant="outline">{{ t('jobUi.llmReadOnly') }}</Badge>
        </div>

        <div class="divide-y divide-border/60 text-xs">
          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">{{ t('jobUi.runtimeReadinessLabel') }}</span>
            <span class="inline-flex items-center gap-1.5 font-semibold">
              <span
                class="size-2 rounded-full"
                :class="settings.runtime_ready ? 'bg-success' : 'bg-warning'"
                aria-hidden="true"
              />
              <span>{{ settings.runtime_ready ? 'Ready' : 'Unavailable' }}</span>
            </span>
          </div>

          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">{{ t('jobUi.runtimeRevisionLabel') }}</span>
            <span class="font-mono font-semibold">v{{ settings.revision }}</span>
          </div>

          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">{{ t('jobUi.runtimeRootProfilesLabel') }}</span>
            <span class="font-mono font-semibold">{{ rootAgentsCount }}</span>
          </div>

          <div class="flex items-center justify-between py-2.5">
            <span class="text-muted-foreground">{{ t('jobUi.runtimeActiveSubagentsLabel') }}</span>
            <span class="font-mono font-semibold">{{ activeSubagentsCount }}</span>
          </div>
        </div>
      </section>
    </template>
  </section>
</template>

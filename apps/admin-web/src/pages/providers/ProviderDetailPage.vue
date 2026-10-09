<script setup lang="ts">
import { Activity, ArrowRight, CircleHelp, FlaskConical, Layers3, Pencil, Play, RefreshCw, RotateCcw, Settings2, Trash2 } from '@lucide/vue'
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { RouterLink, useRoute, useRouter } from 'vue-router'

import { formatApiError, isApiError } from '@/api/errors'
import { providerAdaptersApi } from '@/api/provider-adapters'
import { providersApi } from '@/api/providers'
import type { AdminProvider, ProviderAdapter, ProviderTemplate } from '@/api/types/providers'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import ProviderFormModal from '@/components/admin/ProviderFormModal.vue'
import ProviderTestPanel from '@/components/providers/ProviderTestPanel.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { ProviderInstance, ProviderStatus, ProviderType } from '@/domain/admin'
import { providerTypeIcons } from '@/lib/providerTypeIcons'
import { redactEndpoint } from '@/lib/providerEndpoint'
import { useAdminStore } from '@/stores/admin'

const route = useRoute()
const router = useRouter()
const store = useAdminStore()
const { t, providerTypeLabel } = useI18n()
const key = computed(() => typeof route.params.key === 'string' ? route.params.key : '')

const provider = ref<AdminProvider>()
const adapterDescriptor = ref<ProviderAdapter>()
const bindings = ref<ProviderTemplate[]>([])
const bindingTotal = ref(0)
const bindingsLoaded = ref(false)
const nextBindingPage = ref(2)
const loading = ref(false)
const loadingBindings = ref(false)
const busy = ref(false)
const error = ref('')
const editOpen = ref(false)
const deleteOpen = ref(false)
let loadController: AbortController | undefined
let bindingController: AbortController | undefined

function parseConfig(value: string): Record<string, unknown> {
  try {
    const parsed: unknown = JSON.parse(value)
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed as Record<string, unknown> : {}
  } catch { return {} }
}

const config = computed(() => parseConfig(provider.value?.config_json ?? '{}'))
function stringConfig(field: string) {
  const value = config.value[field]
  return typeof value === 'string' ? value : ''
}
const model = computed(() => stringConfig('model'))
const endpoint = computed(() => redactEndpoint(stringConfig('base_url') || stringConfig('endpoint')))
const enabled = computed(() => provider.value?.enabled === 1)
const runtimeState = computed(() => provider.value?.runtime?.desired_state ?? provider.value?.runtime_status ?? 'not_loaded')
const isRuntimeReady = computed(() =>
  provider.value?.runtime
    ? provider.value.runtime.desired_state === 'ready'
    : provider.value?.runtime_status === 'loaded' && provider.value.runtime_matches_desired,
)
const typeIcon = computed(() => providerTypeIcons[provider.value?.type ?? 'llm'])
const runtimeTone = computed(() => isRuntimeReady.value
  ? 'text-success'
  : runtimeState.value === 'failed' || runtimeState.value === 'quarantined' || runtimeState.value === 'unavailable'
    ? 'text-danger'
    : 'text-warning')

/** This is the current desired configuration, not an inference health check. */
const editableProvider = computed<ProviderInstance | undefined>(() => {
  const item = provider.value
  if (!item) return undefined
  const status: ProviderStatus = !item.enabled ? 'disabled'
    : runtimeState.value === 'failed' || runtimeState.value === 'quarantined' || runtimeState.value === 'unavailable'
      ? 'error' : 'ready'
  return {
    id: item.key,
    name: item.name,
    type: item.type as ProviderType,
    adapter: item.adapter,
    configJson: config.value,
    model: model.value,
    description: stringConfig('description'),
    endpoint: stringConfig('base_url') || stringConfig('endpoint'),
    status,
    desiredRevision: item.revision,
    runtime: item.runtime,
    credential: item.credential,
    credentialEnv: item.credential_env ?? undefined,
  }
})

const sensitiveField = (field: string) => /key|token|password|secret|credential|auth|signature/i.test(field)
const configFields = computed(() => (adapterDescriptor.value?.config_schema?.fields ?? [])
  .filter((field) => !sensitiveField(field.key) && field.key !== 'model' && field.key !== 'base_url' && field.key !== 'endpoint')
  .flatMap((field) => {
    const value = config.value[field.key]
    if (value === undefined || value === null || typeof value === 'object') return []
    return [{ key: field.key, label: field.label || field.key, value: typeof value === 'string' && (value.startsWith('https://') || value.startsWith('http://')) ? redactEndpoint(value) : String(value) }]
  }))

async function load() {
  loadController?.abort()
  bindingController?.abort()
  const pending = new AbortController()
  loadController = pending
  loading.value = true
  error.value = ''
  provider.value = undefined
  adapterDescriptor.value = undefined
  bindings.value = []
  bindingTotal.value = 0
  bindingsLoaded.value = false
  nextBindingPage.value = 2
  try {
    const item = await providersApi.get(key.value, pending.signal)
    if (pending.signal.aborted) return
    provider.value = item
    const descriptor = providerAdaptersApi.get(item.adapter, pending.signal)
      .then((result) => { if (!pending.signal.aborted) adapterDescriptor.value = result })
      .catch(() => { /* Optional schema does not block provider management. */ })
    const page = await providersApi.templates(item.key, 1, 50, pending.signal)
    if (pending.signal.aborted) return
    bindings.value = page.items
    bindingTotal.value = page.total
    bindingsLoaded.value = true
    await descriptor
  } catch (cause) {
    if (!pending.signal.aborted) error.value = formatApiError(cause)
  } finally {
    if (loadController === pending) loading.value = false
  }
}

async function loadMoreBindings() {
  if (!provider.value || loadingBindings.value || bindings.value.length >= bindingTotal.value) return
  bindingController?.abort()
  const pending = new AbortController()
  bindingController = pending
  loadingBindings.value = true
  try {
    const page = await providersApi.templates(provider.value.key, nextBindingPage.value, 50, pending.signal)
    if (pending.signal.aborted) return
    bindings.value = [...bindings.value, ...page.items]
    nextBindingPage.value += 1
    bindingTotal.value = page.total
  } catch (cause) {
    if (!pending.signal.aborted) error.value = formatApiError(cause)
  } finally {
    if (bindingController === pending) loadingBindings.value = false
  }
}

async function mutate(action: () => Promise<unknown>) {
  if (busy.value) return
  busy.value = true
  error.value = ''
  try {
    await action()
    await load()
    await store.refreshAll()
  } catch (cause) {
    if (isApiError(cause) && cause.status === 409) await load()
    error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}

function toggleEnabled() {
  if (!provider.value) return
  const current = provider.value
  void mutate(() => providersApi.update(current.key, { enabled: !enabled.value }, current.revision))
}
function prepare() {
  if (!provider.value || !enabled.value || !provider.value.runtime?.can_prepare) return
  const current = provider.value
  void mutate(() => providersApi.prepare(current.key))
}
function save(payload: {
  name: string
  type: ProviderType
  adapter: string
  model: string
  description: string
  status: ProviderStatus
  endpoint?: string
  apiKey?: string
  configJson?: Record<string, unknown>
}) {
  if (!provider.value) return
  const current = provider.value
  void mutate(() => providersApi.update(current.key, {
    name: payload.name,
    adapter: payload.adapter,
    config_json: payload.configJson ?? config.value,
    enabled: payload.status !== 'disabled',
    ...(payload.apiKey ? { api_key: payload.apiKey } : {}),
  }, current.revision))
}
async function remove() {
  if (!provider.value || !bindingsLoaded.value || bindingTotal.value > 0 || busy.value) return
  const current = provider.value
  busy.value = true
  error.value = ''
  try {
    await providersApi.remove(current.key, current.revision)
    await store.refreshAll()
    await router.replace('/providers')
  } catch (cause) {
    if (isApiError(cause) && cause.status === 409) await load()
    error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}

watch(key, () => {
  editOpen.value = false
  deleteOpen.value = false
  void load()
}, { immediate: true })
onBeforeUnmount(() => {
  loadController?.abort()
  bindingController?.abort()
})
</script>

<template>
  <main class="mx-auto max-w-7xl space-y-5">
    <DetailHeader v-if="provider" :title="provider.name" :back-label="t('providers.title')" @back="router.push('/providers')">
      <template #icon>
        <span class="flex size-10 shrink-0 items-center justify-center rounded-xl border border-studio-violet/20 bg-studio-violet/10 text-studio-violet">
          <component :is="typeIcon" class="size-5" aria-hidden="true" />
        </span>
      </template>
      <template #details>
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span class="font-medium">{{ providerTypeLabel(provider.type) }}</span>
          <span aria-hidden="true">·</span>
          <span class="font-mono">{{ provider.adapter }}</span>
          <span aria-hidden="true">·</span>
          <span class="break-all font-mono">{{ provider.key }}</span>
        </div>
      </template>
      <template #actions>
        <Button variant="outline" size="sm" :disabled="busy" @click="load">
          <RefreshCw class="size-4" aria-hidden="true" />{{ t('common.refresh') }}
        </Button>
        <Button variant="outline" size="sm" :disabled="busy" @click="editOpen = true">
          <Pencil class="size-4" aria-hidden="true" />{{ t('common.edit') }}
        </Button>
        <Button variant="outline" size="sm" :disabled="busy" @click="toggleEnabled">
          {{ enabled ? t('providerDetail.disable') : t('providerDetail.enable') }}
        </Button>
        <Button variant="ghost" size="sm" class="text-danger" :disabled="busy || !bindingsLoaded || bindingTotal > 0" @click="deleteOpen = true">
          <Trash2 class="size-4" aria-hidden="true" />{{ t('common.delete') }}
        </Button>
      </template>
    </DetailHeader>

    <p v-if="error" role="alert" class="rounded-lg border border-danger/30 bg-danger/5 px-4 py-3 text-sm text-danger">{{ error }}</p>
    <div v-if="loading && !provider" class="studio-panel p-10 text-center text-muted-foreground" role="status">
      {{ t('common.loading') }}
    </div>
    <div v-if="!loading && !provider" class="space-y-3">
      <RouterLink to="/providers" class="text-sm text-studio-violet hover:underline">← {{ t('providers.title') }}</RouterLink>
      <p class="text-sm text-muted-foreground">{{ t('providerDetail.unavailable') }}</p>
      <Button variant="outline" @click="load">{{ t('common.retry') }}</Button>
    </div>

    <template v-if="provider">
      <div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-4" aria-label="Provider summary">
        <section class="studio-panel px-4 py-3">
          <p class="text-xs text-muted-foreground">{{ t('providerDetail.configurationState') }}</p>
          <div class="mt-2 flex items-center gap-2 font-semibold">
            <span class="size-2 rounded-full" :class="enabled ? 'bg-success' : 'bg-muted-foreground'" aria-hidden="true" />
            {{ enabled ? t('providerDetail.enabled') : t('providerDetail.disabled') }}
          </div>
          <p class="mt-1 text-xs text-muted-foreground">{{ t('providerDetail.enabledHint') }}</p>
        </section>
        <section class="studio-panel px-4 py-3">
          <p class="text-xs text-muted-foreground">{{ t('diagnostics.runtime') }}</p>
          <div class="mt-2 flex items-center gap-2 font-semibold" :class="runtimeTone">
            <Activity class="size-4" aria-hidden="true" />
            <span class="capitalize">{{ runtimeState.replaceAll('_', ' ') }}</span>
          </div>
          <p class="mt-1 text-xs text-muted-foreground">{{ t('providerDetail.runtimeHint') }}</p>
        </section>
        <section class="studio-panel px-4 py-3">
          <p class="text-xs text-muted-foreground">{{ t('providers.templateUsage') }}</p>
          <p class="mt-2 text-lg font-semibold tabular-nums">{{ bindingTotal }}</p>
          <p class="mt-1 text-xs text-muted-foreground">{{ t('providerDetail.bindingsHint') }}</p>
        </section>
        <section class="studio-panel px-4 py-3">
          <p class="text-xs text-muted-foreground">{{ t('providerDetail.desiredRevision') }}</p>
          <p class="mt-2 font-mono text-lg font-semibold tabular-nums">r{{ provider.revision }}</p>
          <p class="mt-1 text-xs text-muted-foreground">{{ t('providerDetail.revisionHint') }}</p>
        </section>
      </div>

      <div class="grid items-start gap-4 lg:grid-cols-[minmax(0,1.35fr)_minmax(300px,1fr)]">
        <div class="min-w-0 space-y-4">
          <section class="studio-panel space-y-4 p-4 sm:p-5" aria-labelledby="provider-test-heading">
            <div class="flex items-start gap-3">
              <span class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-studio-violet/10 text-studio-violet">
                <FlaskConical class="size-4" aria-hidden="true" />
              </span>
              <div>
                <h2 id="provider-test-heading" class="font-semibold">{{ t('providers.testTitle') }}</h2>
                <p class="mt-1 text-xs text-muted-foreground">{{ t('providerDetail.testHint') }}</p>
              </div>
            </div>
            <ProviderTestPanel
              :key="`${provider.key}:${provider.revision}`"
              :type="provider.type"
              :adapter="provider.adapter"
              :revision="provider.revision"
              :saved-key="provider.key"
              :capabilities="adapterDescriptor?.capabilities"
            />
          </section>

          <section class="studio-panel space-y-4 p-4 sm:p-5" aria-labelledby="provider-runtime-heading">
            <div class="flex flex-wrap items-start justify-between gap-3">
              <div class="flex items-center gap-2">
                <Activity class="size-4 text-studio-violet" aria-hidden="true" />
                <div>
                  <h2 id="provider-runtime-heading" class="font-semibold">{{ t('providerDetail.runtimeLifecycle') }}</h2>
                  <p class="mt-1 text-xs text-muted-foreground">{{ t('providerDetail.runtimeNote') }}</p>
                </div>
              </div>
              <Button
                size="sm"
                variant="outline"
                :disabled="busy || !enabled || !provider.runtime?.can_prepare"
                @click="prepare"
              >
                <Play class="size-4" aria-hidden="true" />{{ t('providerDetail.prepare') }}
              </Button>
            </div>
            <div class="grid gap-3 rounded-lg border border-border/70 bg-surface/60 p-3 text-sm sm:grid-cols-2">
              <div>
                <p class="text-xs text-muted-foreground">{{ t('providerDetail.desiredState') }}</p>
                <p class="mt-1 font-mono">{{ runtimeState }}</p>
              </div>
              <div>
                <p class="text-xs text-muted-foreground">{{ t('providerDetail.readyRevisions') }}</p>
                <p class="mt-1 font-mono">{{ provider.runtime?.ready_revisions.join(', ') || '—' }}</p>
              </div>
              <div>
                <p class="text-xs text-muted-foreground">{{ t('providerDetail.runtimeMatch') }}</p>
                <p class="mt-1">{{ provider.runtime_matches_desired ? t('providerDetail.matches') : t('providerDetail.notReady') }}</p>
              </div>
              <div>
                <p class="text-xs text-muted-foreground">{{ t('providerDetail.failureCode') }}</p>
                <p class="mt-1 break-all font-mono">{{ provider.runtime?.failure_code || '—' }}</p>
              </div>
            </div>
            <p v-if="provider.requires_restart" class="flex items-start gap-2 text-xs text-warning">
              <RotateCcw class="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
              {{ t('providerDetail.restartRequired') }}
            </p>
            <p v-else-if="!enabled" class="text-xs text-muted-foreground">{{ t('providerDetail.disabledPrepare') }}</p>
            <p v-else-if="!provider.runtime?.can_prepare && !isRuntimeReady" class="text-xs text-muted-foreground">{{ t('providerDetail.prepareUnavailable') }}</p>
          </section>
        </div>

        <div class="min-w-0 space-y-4">
          <section class="studio-panel space-y-4 p-4 sm:p-5" aria-labelledby="provider-config-heading">
            <div class="flex items-center gap-2">
              <Settings2 class="size-4 text-studio-violet" aria-hidden="true" />
              <h2 id="provider-config-heading" class="font-semibold">{{ t('providerDetail.configurationTitle') }}</h2>
            </div>
            <p v-if="stringConfig('description')" class="text-sm text-muted-foreground">{{ stringConfig('description') }}</p>
            <dl class="space-y-3 text-sm">
              <div class="grid grid-cols-[110px_minmax(0,1fr)] gap-3">
                <dt class="text-xs text-muted-foreground">{{ t('providers.type') }}</dt>
                <dd>{{ providerTypeLabel(provider.type) }}</dd>
              </div>
              <div class="grid grid-cols-[110px_minmax(0,1fr)] gap-3">
                <dt class="text-xs text-muted-foreground">{{ t('providers.adapter') }}</dt>
                <dd class="break-all font-mono text-xs">{{ provider.adapter }}</dd>
              </div>
              <div class="grid grid-cols-[110px_minmax(0,1fr)] gap-3">
                <dt class="text-xs text-muted-foreground">{{ t('providers.model') }}</dt>
                <dd class="break-all font-mono text-xs">{{ model || t('providers.modelMissing') }}</dd>
              </div>
              <div v-if="endpoint" class="grid grid-cols-[110px_minmax(0,1fr)] gap-3">
                <dt class="text-xs text-muted-foreground">{{ t('providers.endpoint') }}</dt>
                <dd class="break-all font-mono text-xs">{{ endpoint }}</dd>
              </div>
              <div v-for="field in configFields" :key="field.key" class="grid grid-cols-[110px_minmax(0,1fr)] gap-3">
                <dt class="break-words text-xs text-muted-foreground">{{ field.label }}</dt>
                <dd class="break-all font-mono text-xs">{{ field.value }}</dd>
              </div>
            </dl>
            <p class="flex items-center gap-1.5 border-t pt-3 text-xs text-muted-foreground">
              <CircleHelp class="size-3.5 shrink-0" aria-hidden="true" />
              {{ t('providerDetail.secretHint') }}
            </p>
            <Button size="sm" variant="outline" :disabled="busy" @click="editOpen = true">
              <Pencil class="size-3.5" aria-hidden="true" />{{ t('common.edit') }}
            </Button>
          </section>

          <section class="studio-panel space-y-3 p-4 sm:p-5" aria-labelledby="provider-bindings-heading">
            <div class="flex items-center justify-between gap-2">
              <div class="flex items-center gap-2">
                <Layers3 class="size-4 text-studio-violet" aria-hidden="true" />
                <h2 id="provider-bindings-heading" class="font-semibold">{{ t('providers.templateUsage') }}</h2>
              </div>
              <span class="text-xs tabular-nums text-muted-foreground">{{ bindingTotal }}</span>
            </div>
            <p class="text-xs text-muted-foreground">{{ t('providerDetail.bindingDescription') }}</p>
            <p v-if="!bindingTotal" class="rounded-lg border border-dashed p-4 text-sm text-muted-foreground">
              {{ t('providers.usageEmpty') }}
            </p>
            <ul v-else class="divide-y divide-border/70 rounded-lg border border-border/70">
              <li v-for="binding in bindings" :key="binding.key">
                <RouterLink :to="`/templates/${encodeURIComponent(binding.key)}`" class="group flex items-center justify-between gap-2 p-3 hover:bg-accent/40">
                  <span class="min-w-0">
                    <span class="block truncate text-sm font-medium">{{ binding.name }}</span>
                    <span class="mt-0.5 block truncate text-xs text-muted-foreground">{{ binding.key }}</span>
                  </span>
                  <ArrowRight class="size-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" aria-hidden="true" />
                </RouterLink>
              </li>
            </ul>
            <Button v-if="bindings.length < bindingTotal" variant="outline" size="sm" :disabled="loadingBindings" @click="loadMoreBindings">
              {{ t('providerDetail.loadMore') }}
            </Button>
            <p v-if="bindingTotal > 0" class="text-xs text-muted-foreground">{{ t('providerDetail.deleteGuard') }}</p>
          </section>
        </div>
      </div>

      <ProviderFormModal
        v-model="editOpen"
        :provider="editableProvider"
        :providers="store.providers"
        :usage-count="bindingTotal"
        @save="save"
      />
      <ConfirmDialog
        v-model="deleteOpen"
        tone="danger"
        :title="t('providerDelete.title', { name: provider.name })"
        :description="t('providerDelete.description')"
        :confirm-label="t('providerDelete.submit')"
        @confirm="remove"
      >
        {{ t('providerDelete.unused') }}
      </ConfirmDialog>
    </template>
  </main>
</template>

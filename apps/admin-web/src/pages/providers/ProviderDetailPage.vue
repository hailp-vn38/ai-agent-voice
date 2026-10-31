<script setup lang="ts">
import {
  Activity,
  FlaskConical,
  Layers3,
  LayoutGrid,
  LayoutTemplate,
  Link2,
  Pencil,
  Play,
  RefreshCw,
  Settings2,
  ShieldAlert,
  Trash2,
} from '@lucide/vue'
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
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import {
  effectiveLlmTags,
  type ProviderInstance,
  type ProviderStatus,
  type ProviderType,
} from '@/domain/admin'
import { redactEndpoint } from '@/lib/providerEndpoint'
import { providerTypeIcons } from '@/lib/providerTypeIcons'
import { useAdminStore } from '@/stores/admin'

type DetailTab = 'overview' | 'config' | 'runtime' | 'test' | 'bindings'

const route = useRoute()
const router = useRouter()
const catalogRoute = computed(() => ({ path: '/providers', query: route.query }))
const store = useAdminStore()
const { t, providerTypeLabel, providerStatusLabel } = useI18n()
const key = computed(() => (typeof route.params.key === 'string' ? route.params.key : ''))

const activeTab = ref<DetailTab>('overview')
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
const isConfigProvider = computed(() => provider.value?.source === 'config')
const editOpen = ref(false)
const deleteOpen = ref(false)
let loadController: AbortController | undefined
let bindingController: AbortController | undefined

const lifecycleList = [
  { id: 'cold', label: 'Initial' },
  { id: 'queued', label: 'Queued' },
  { id: 'loading', label: 'Loading' },
  { id: 'ready', label: 'Ready' },
  { id: 'failed', label: 'Failed' },
  { id: 'draining', label: 'Draining' },
  { id: 'quarantined', label: 'Quarantined' },
] as const

function parseConfig(value: string): Record<string, unknown> {
  try {
    const parsed: unknown = JSON.parse(value)
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : {}
  } catch {
    return {}
  }
}

const config = computed(() => parseConfig(provider.value?.config_json ?? '{}'))
function stringConfig(field: string) {
  const value = config.value[field]
  return typeof value === 'string' ? value : ''
}
const model = computed(() => stringConfig('model'))
const endpoint = computed(() => redactEndpoint(stringConfig('base_url') || stringConfig('endpoint')))
const enabled = computed(() => provider.value?.enabled === 1)
const runtimeState = computed(
  () => provider.value?.runtime?.desired_state ?? provider.value?.runtime_status ?? 'not_loaded',
)
const isRuntimeReady = computed(() =>
  provider.value?.runtime
    ? provider.value.runtime.desired_state === 'ready'
    : provider.value?.runtime_status === 'loaded' && provider.value.runtime_matches_desired,
)
const typeIcon = computed(() => providerTypeIcons[provider.value?.type ?? 'llm'])
const runtimeTone = computed(() =>
  isRuntimeReady.value
    ? 'text-success'
    : runtimeState.value === 'failed' ||
        runtimeState.value === 'quarantined' ||
        runtimeState.value === 'unavailable'
      ? 'text-danger'
      : 'text-warning',
)

const status = computed<ProviderStatus>(() => {
  if (!enabled.value) return 'disabled'
  if (
    runtimeState.value === 'failed' ||
    runtimeState.value === 'quarantined' ||
    runtimeState.value === 'unavailable'
  ) {
    return 'error'
  }
  return 'ready'
})

const typeIconBoxClass = computed(() => {
  switch (provider.value?.type) {
    case 'asr':
      return 'border-sky-500/25 bg-sky-500/12 text-sky-400'
    case 'tts':
      return 'border-emerald-500/25 bg-emerald-500/12 text-emerald-400'
    case 'vad':
      return 'border-amber-500/25 bg-amber-500/12 text-amber-400'
    case 'llm':
      return 'border-violet-500/25 bg-violet-500/12 text-violet-400'
    default:
      return 'border-primary/25 bg-primary/12 text-primary'
  }
})

const statusChipClass = computed(() => {
  if (status.value === 'error') return 'border-danger/30 bg-danger/10 text-danger'
  if (status.value === 'disabled') return 'border-border/70 bg-muted/40 text-muted-foreground'
  return 'border-emerald-500/30 bg-emerald-500/10 text-emerald-400'
})

const statusDotClass = computed(() => {
  if (status.value === 'error') return 'bg-danger'
  if (status.value === 'disabled') return 'bg-muted-foreground'
  return 'bg-emerald-400 shadow-[0_0_0_3px_rgba(16,185,129,0.14)]'
})

/** This is the current desired configuration, not an inference health check. */
const editableProvider = computed<ProviderInstance | undefined>(() => {
  const item = provider.value
  if (!item) return undefined
  return {
    id: item.key,
    source: item.source,
    name: item.name,
    type: item.type as ProviderType,
    adapter: item.adapter,
    configJson: config.value,
    model: model.value,
    description: stringConfig('description'),
    endpoint: stringConfig('base_url') || stringConfig('endpoint'),
    status: status.value,
    desiredRevision: item.revision,
    runtime: item.runtime,
    credential: item.credential,
    credentialEnv: item.credential_env ?? undefined,
  }
})

const sensitiveField = (field: string) =>
  /key|token|password|secret|credential|auth|signature/i.test(field)
const configFields = computed(() =>
  (adapterDescriptor.value?.config_schema?.fields ?? [])
    .filter(
      (field) =>
        !sensitiveField(field.key) &&
        field.key !== 'model' &&
        field.key !== 'base_url' &&
        field.key !== 'endpoint',
    )
    .flatMap((field) => {
      const rawValue = config.value[field.key]
      const value =
        rawValue === undefined && field.key === 'tags' && provider.value?.type === 'llm'
          ? effectiveLlmTags({ type: 'llm', configJson: config.value })
          : rawValue
      if (value === undefined || value === null) return []
      if (Array.isArray(value)) {
        const items = value.map(String).filter(Boolean)
        return items.length
          ? [{ key: field.key, label: field.label || field.key, value: items.join(', ') }]
          : []
      }
      if (typeof value === 'object') return []
      return [
        {
          key: field.key,
          label: field.label || field.key,
          value:
            typeof value === 'string' &&
            (value.startsWith('https://') || value.startsWith('http://'))
              ? redactEndpoint(value)
              : String(value),
        },
      ]
    }),
)

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
    const descriptor = providerAdaptersApi
      .get(item.adapter, pending.signal)
      .then((result) => {
        if (!pending.signal.aborted) adapterDescriptor.value = result
      })
      .catch(() => {
        /* Optional schema does not block provider management. */
      })
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
  if (!provider.value || loadingBindings.value || bindings.value.length >= bindingTotal.value)
    return
  bindingController?.abort()
  const pending = new AbortController()
  bindingController = pending
  loadingBindings.value = true
  try {
    const page = await providersApi.templates(
      provider.value.key,
      nextBindingPage.value,
      50,
      pending.signal,
    )
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
  if (!provider.value || isConfigProvider.value) return
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
  apiKey?: string | null
  configJson?: Record<string, unknown>
}) {
  if (!provider.value || isConfigProvider.value) return
  const current = provider.value
  const nextConfig = payload.configJson ?? config.value
  const hasGatewayBinding =
    payload.adapter === 'llm_gateway' ||
    (typeof nextConfig.gateway_binding === 'string' && nextConfig.gateway_binding.trim().length > 0)
  void mutate(() =>
    providersApi.update(
      current.key,
      {
        name: payload.name,
        adapter: payload.adapter,
        config_json: nextConfig,
        enabled: payload.status !== 'disabled',
        ...(hasGatewayBinding || payload.apiKey === null
          ? { api_key: null }
          : payload.apiKey
            ? { api_key: payload.apiKey }
            : {}),
      },
      current.revision,
    ),
  )
}
async function remove() {
  if (
    !provider.value ||
    isConfigProvider.value ||
    !bindingsLoaded.value ||
    bindingTotal.value > 0 ||
    busy.value
  )
    return
  const current = provider.value
  busy.value = true
  error.value = ''
  try {
    await providersApi.remove(current.key, current.revision)
    await store.refreshAll()
    await router.replace(catalogRoute.value)
  } catch (cause) {
    if (isApiError(cause) && cause.status === 409) await load()
    error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}

watch(
  key,
  () => {
    activeTab.value = 'overview'
    editOpen.value = false
    deleteOpen.value = false
    void load()
  },
  { immediate: true },
)
onBeforeUnmount(() => {
  loadController?.abort()
  bindingController?.abort()
})
</script>

<template>
  <div class="space-y-4">
    <DetailHeader
      :title="provider?.name || key || t('providers.detailTitle')"
      :back-label="t('providers.title')"
      @back="router.push(catalogRoute)"
    >
      <template #icon>
        <div
          class="flex size-9 shrink-0 items-center justify-center rounded-lg border"
          :class="typeIconBoxClass"
        >
          <component :is="typeIcon" class="size-4" aria-hidden="true" />
        </div>
      </template>

      <template #details>
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span v-if="provider" class="font-medium uppercase">
            {{ providerTypeLabel(provider.type) }}
          </span>
          <span v-if="provider" class="font-mono">· {{ provider.adapter }}</span>
          <span v-if="provider" class="font-mono">· {{ provider.key }}</span>
          <Badge v-if="isConfigProvider" variant="outline">Config</Badge>
        </div>
      </template>

      <template #actions>
        <span
          v-if="provider"
          class="inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs font-medium"
          :class="statusChipClass"
        >
          <span class="size-1.5 rounded-full" :class="statusDotClass" aria-hidden="true" />
          {{ providerStatusLabel(status) }}
        </span>

        <Button
          type="button"
          variant="outline"
          size="sm"
          :disabled="loading || busy"
          @click="load"
        >
          <RefreshCw class="size-3.5" aria-hidden="true" />
          {{ t('common.refresh') }}
        </Button>

        <Button
          v-if="provider"
          type="button"
          variant="outline"
          size="sm"
          :disabled="busy || isConfigProvider"
          @click="editOpen = true"
        >
          <Pencil class="size-3.5" aria-hidden="true" />
          {{ t('common.edit') }}
        </Button>

        <Button
          v-if="provider"
          type="button"
          variant="outline"
          size="sm"
          :disabled="busy || isConfigProvider"
          @click="toggleEnabled"
        >
          {{ enabled ? t('providerDetail.disable') : t('providerDetail.enable') }}
        </Button>

        <Button
          v-if="provider"
          type="button"
          variant="outline"
          size="sm"
          class="text-danger"
          :disabled="busy || isConfigProvider || !bindingsLoaded || bindingTotal > 0"
          @click="deleteOpen = true"
        >
          <Trash2 class="size-3.5" aria-hidden="true" />
          {{ t('common.delete') }}
        </Button>
      </template>
    </DetailHeader>

    <p v-if="error" role="alert" class="studio-panel p-4 text-sm text-danger">
      {{ error }}
    </p>
    <p v-else-if="loading" class="studio-panel p-4 text-sm text-muted-foreground">
      {{ t('common.loading') }}
    </p>

    <template v-else-if="provider">
      <!-- Variant A Horizontal Detail Tabs -->
      <nav
        class="flex flex-wrap items-center gap-1 border-b border-border/70 pb-2"
        aria-label="Provider detail sections"
      >
        <button
          type="button"
          class="inline-flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-xs font-semibold transition-colors"
          :class="
            activeTab === 'overview'
              ? 'border-primary/40 bg-primary/12 text-foreground'
              : 'border-transparent text-muted-foreground hover:bg-muted/40 hover:text-foreground'
          "
          @click="activeTab = 'overview'"
        >
          <LayoutGrid class="size-3.5" aria-hidden="true" />
          {{ t('providerDetail.tab.overview') }}
        </button>

        <button
          type="button"
          class="inline-flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-xs font-semibold transition-colors"
          :class="
            activeTab === 'config'
              ? 'border-primary/40 bg-primary/12 text-foreground'
              : 'border-transparent text-muted-foreground hover:bg-muted/40 hover:text-foreground'
          "
          @click="activeTab = 'config'"
        >
          <Settings2 class="size-3.5" aria-hidden="true" />
          {{ t('providerDetail.tab.config') }}
        </button>

        <button
          type="button"
          class="inline-flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-xs font-semibold transition-colors"
          :class="
            activeTab === 'runtime'
              ? 'border-primary/40 bg-primary/12 text-foreground'
              : 'border-transparent text-muted-foreground hover:bg-muted/40 hover:text-foreground'
          "
          @click="activeTab = 'runtime'"
        >
          <Activity class="size-3.5" aria-hidden="true" />
          {{ t('providerDetail.tab.runtime') }}
        </button>

        <button
          type="button"
          class="inline-flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-xs font-semibold transition-colors"
          :class="
            activeTab === 'test'
              ? 'border-primary/40 bg-primary/12 text-foreground'
              : 'border-transparent text-muted-foreground hover:bg-muted/40 hover:text-foreground'
          "
          @click="activeTab = 'test'"
        >
          <FlaskConical class="size-3.5" aria-hidden="true" />
          {{ t('providerDetail.tab.test') }}
        </button>

        <button
          type="button"
          class="inline-flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-xs font-semibold transition-colors"
          :class="
            activeTab === 'bindings'
              ? 'border-primary/40 bg-primary/12 text-foreground'
              : 'border-transparent text-muted-foreground hover:bg-muted/40 hover:text-foreground'
          "
          @click="activeTab = 'bindings'"
        >
          <Link2 class="size-3.5" aria-hidden="true" />
          {{ t('providerDetail.tab.bindings') }}
          <span class="rounded-full bg-muted px-1.5 py-0.2 text-[10px] tabular-nums text-muted-foreground">
            {{ bindingTotal }}
          </span>
        </button>
      </nav>

      <!-- TAB 1: OVERVIEW -->
      <div v-show="activeTab === 'overview'" class="space-y-4">
        <!-- 4 Summary KPI Cards -->
        <section class="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <div class="studio-panel flex items-center gap-3 p-3.5">
            <span class="flex size-8 shrink-0 items-center justify-center rounded-lg border border-border/70 bg-muted/30 text-muted-foreground">
              <Settings2 class="size-4" aria-hidden="true" />
            </span>
            <div class="min-w-0">
              <p class="text-sm font-semibold" :class="enabled ? 'text-success' : 'text-muted-foreground'">
                {{ enabled ? t('providerDetail.enabled') : t('providerDetail.disabled') }}
              </p>
              <p class="truncate text-xs text-muted-foreground">
                {{ t('providerDetail.configurationState') }}
              </p>
            </div>
          </div>

          <div class="studio-panel flex items-center gap-3 p-3.5">
            <span class="flex size-8 shrink-0 items-center justify-center rounded-lg border border-border/70 bg-muted/30 text-muted-foreground">
              <Activity class="size-4" aria-hidden="true" />
            </span>
            <div class="min-w-0">
              <p class="font-mono text-sm font-semibold" :class="runtimeTone">{{ runtimeState }}</p>
              <p class="truncate text-xs text-muted-foreground">
                {{ t('providers.status') }}
              </p>
            </div>
          </div>

          <div class="studio-panel flex items-center gap-3 p-3.5">
            <span class="flex size-8 shrink-0 items-center justify-center rounded-lg border border-border/70 bg-muted/30 text-muted-foreground">
              <Link2 class="size-4" aria-hidden="true" />
            </span>
            <div class="min-w-0">
              <p class="text-sm font-semibold tabular-nums">{{ bindingTotal }}</p>
              <p class="truncate text-xs text-muted-foreground">
                {{ t('providers.templateUsage') }}
              </p>
            </div>
          </div>

          <div class="studio-panel flex items-center gap-3 p-3.5">
            <span class="flex size-8 shrink-0 items-center justify-center rounded-lg border border-border/70 bg-muted/30 text-muted-foreground">
              <Layers3 class="size-4" aria-hidden="true" />
            </span>
            <div class="min-w-0">
              <p class="font-mono text-sm font-semibold">
                r{{ provider.runtime?.desired_revision ?? provider.revision }}
              </p>
              <p class="truncate text-xs text-muted-foreground">
                {{ t('providerDetail.desiredRevision') }}
              </p>
            </div>
          </div>
        </section>

        <!-- Overview 2-Column Grid -->
        <div class="grid gap-4 lg:grid-cols-[minmax(0,1.15fr)_minmax(0,0.85fr)]">
          <div class="space-y-4">
            <!-- Provider Overview Card -->
            <section class="studio-panel space-y-3 p-4">
              <div class="flex items-center justify-between gap-2 border-b border-border/60 pb-2.5">
                <h2 class="text-sm font-semibold">{{ t('providerDetail.overviewCard') }}</h2>
                <Button type="button" size="sm" variant="outline" @click="activeTab = 'config'">
                  <Settings2 class="size-3.5" aria-hidden="true" />
                  {{ t('providerDetail.tab.config') }}
                </Button>
              </div>

              <p v-if="stringConfig('description')" class="text-xs leading-relaxed text-muted-foreground">
                {{ stringConfig('description') }}
              </p>

              <dl class="divide-y divide-border/50 text-xs">
                <div class="flex items-center justify-between gap-3 py-2">
                  <dt class="text-muted-foreground">Key</dt>
                  <dd class="font-mono">{{ provider.key }}</dd>
                </div>
                <div class="flex items-center justify-between gap-3 py-2">
                  <dt class="text-muted-foreground">{{ t('providers.type') }}</dt>
                  <dd>{{ providerTypeLabel(provider.type) }}</dd>
                </div>
                <div class="flex items-center justify-between gap-3 py-2">
                  <dt class="text-muted-foreground">{{ t('providers.adapter') }}</dt>
                  <dd class="font-mono">{{ provider.adapter }}</dd>
                </div>
                <div class="flex items-center justify-between gap-3 py-2">
                  <dt class="text-muted-foreground">{{ t('providers.model') }}</dt>
                  <dd class="font-mono">{{ model || t('providers.modelMissing') }}</dd>
                </div>
                <div class="flex items-center justify-between gap-3 py-2">
                  <dt class="text-muted-foreground">{{ t('providers.endpoint') }}</dt>
                  <dd class="max-w-[60%] truncate font-mono" :title="endpoint || '—'">
                    {{ endpoint || '—' }}
                  </dd>
                </div>
                <div class="flex items-center justify-between gap-3 py-2">
                  <dt class="text-muted-foreground">Source</dt>
                  <dd class="font-mono">{{ isConfigProvider ? 'Config (read-only)' : 'Database' }}</dd>
                </div>
              </dl>
            </section>

            <!-- Quick Actions Card -->
            <section class="studio-panel space-y-2.5 p-4">
              <h2 class="text-sm font-semibold">{{ t('providerDetail.quickActions') }}</h2>
              <div class="flex flex-wrap items-center gap-2">
                <Button type="button" size="sm" @click="activeTab = 'test'">
                  <FlaskConical class="size-3.5" aria-hidden="true" />
                  {{ t('providerDetail.tab.test') }}
                </Button>
                <Button type="button" size="sm" variant="outline" @click="activeTab = 'runtime'">
                  <Activity class="size-3.5" aria-hidden="true" />
                  {{ t('providerDetail.tab.runtime') }}
                </Button>
                <Button type="button" size="sm" variant="outline" @click="activeTab = 'bindings'">
                  <Link2 class="size-3.5" aria-hidden="true" />
                  {{ t('providerDetail.tab.bindings') }}
                </Button>
              </div>
            </section>
          </div>

          <div class="space-y-4">
            <!-- Runtime Snapshot Card -->
            <section class="studio-panel space-y-3 p-4">
              <div class="flex items-center justify-between gap-2 border-b border-border/60 pb-2.5">
                <h2 class="text-sm font-semibold">{{ t('providerDetail.runtimeSnapshot') }}</h2>
                <span
                  class="inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] font-medium"
                  :class="statusChipClass"
                >
                  <span class="size-1.5 rounded-full" :class="statusDotClass" aria-hidden="true" />
                  {{ providerStatusLabel(status) }}
                </span>
              </div>

              <dl class="divide-y divide-border/50 text-xs">
                <div class="flex items-center justify-between gap-3 py-2">
                  <dt class="text-muted-foreground">{{ t('providerDetail.desiredState') }}</dt>
                  <dd class="font-mono font-semibold" :class="runtimeTone">{{ runtimeState }}</dd>
                </div>
                <div class="flex items-center justify-between gap-3 py-2">
                  <dt class="text-muted-foreground">{{ t('providerDetail.readyRevisions') }}</dt>
                  <dd class="font-mono">
                    {{
                      provider.runtime?.ready_revisions?.length
                        ? provider.runtime.ready_revisions.map((item) => `r${item}`).join(', ')
                        : '—'
                    }}
                  </dd>
                </div>
                <div class="flex items-center justify-between gap-3 py-2">
                  <dt class="text-muted-foreground">{{ t('providerDetail.runtimeMatch') }}</dt>
                  <dd>
                    {{
                      isRuntimeReady
                        ? t('providerDetail.matches')
                        : t('providerDetail.notReady')
                    }}
                  </dd>
                </div>
                <div class="flex items-center justify-between gap-3 py-2">
                  <dt class="text-muted-foreground">{{ t('providerDetail.failureCode') }}</dt>
                  <dd class="font-mono text-danger">
                    {{ provider.runtime?.failure_code || '—' }}
                  </dd>
                </div>
              </dl>

              <p v-if="provider.requires_restart" class="text-xs text-warning">
                {{ t('providerDetail.restartRequired') }}
              </p>

              <div class="pt-1">
                <Button
                  type="button"
                  size="sm"
                  :disabled="busy || !enabled || !provider.runtime?.can_prepare"
                  @click="prepare"
                >
                  <Play class="size-3.5" aria-hidden="true" />
                  {{ t('providerDetail.prepare') }}
                </Button>
              </div>
            </section>

            <!-- Bound Templates Preview Card -->
            <section class="studio-panel space-y-3 p-4">
              <div class="flex items-center justify-between gap-2 border-b border-border/60 pb-2.5">
                <h2 class="text-sm font-semibold">{{ t('providers.templateUsage') }}</h2>
                <span class="rounded-full border border-border/70 bg-muted/40 px-2 py-0.5 text-[11px] font-medium tabular-nums text-muted-foreground">
                  {{ bindingTotal }}
                </span>
              </div>

              <p v-if="!bindings.length" class="text-xs text-muted-foreground">
                {{ t('providers.usageEmpty') }}
              </p>

              <ul v-else class="divide-y divide-border/50 text-xs">
                <li v-for="item in bindings.slice(0, 4)" :key="item.key" class="py-2">
                  <RouterLink
                    :to="`/templates/${item.key}`"
                    class="flex items-center justify-between gap-3 rounded transition-colors hover:text-primary"
                  >
                    <span class="flex min-w-0 items-center gap-2">
                      <LayoutTemplate class="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                      <span class="truncate font-medium">{{ item.name }}</span>
                    </span>
                    <span class="shrink-0 font-mono text-[11px] text-muted-foreground">{{ item.key }}</span>
                  </RouterLink>
                </li>
              </ul>
            </section>
          </div>
        </div>
      </div>

      <!-- TAB 2: CONFIGURATION -->
      <div
        v-show="activeTab === 'config'"
        class="grid gap-4 lg:grid-cols-[minmax(0,1.2fr)_minmax(0,0.8fr)]"
      >
        <section class="studio-panel space-y-3 p-4">
          <div class="flex items-start justify-between gap-3 border-b border-border/60 pb-2.5">
            <div>
              <h2 class="text-sm font-semibold">{{ t('providerDetail.configurationTitle') }}</h2>
              <p class="mt-0.5 text-xs text-muted-foreground">{{ t('providerDetail.secretHint') }}</p>
            </div>
            <div class="flex shrink-0 items-center gap-2">
              <span
                class="rounded-full border px-2 py-0.5 text-[11px] font-medium"
                :class="
                  isConfigProvider
                    ? 'border-border/70 bg-muted/40 text-muted-foreground'
                    : 'border-primary/30 bg-primary/10 text-primary'
                "
              >
                {{ isConfigProvider ? 'Read only' : 'Editable' }}
              </span>
              <Button
                type="button"
                variant="outline"
                size="sm"
                :disabled="busy || isConfigProvider"
                @click="editOpen = true"
              >
                <Pencil class="size-3.5" aria-hidden="true" />
                {{ t('common.edit') }}
              </Button>
            </div>
          </div>

          <dl class="grid gap-2.5 sm:grid-cols-2 text-xs">
            <div class="rounded-lg border border-border/60 bg-muted/20 p-2.5">
              <dt class="text-muted-foreground">{{ t('providers.model') }}</dt>
              <dd class="mt-1 break-all font-mono">{{ model || t('providers.modelMissing') }}</dd>
            </div>
            <div class="rounded-lg border border-border/60 bg-muted/20 p-2.5">
              <dt class="text-muted-foreground">{{ t('providers.endpoint') }}</dt>
              <dd class="mt-1 break-all font-mono">{{ endpoint || '—' }}</dd>
            </div>
            <div
              v-for="field in configFields"
              :key="field.key"
              class="rounded-lg border border-border/60 bg-muted/20 p-2.5"
            >
              <dt class="text-muted-foreground">{{ field.label }}</dt>
              <dd class="mt-1 break-all font-mono">{{ field.value }}</dd>
            </div>
          </dl>
        </section>

        <div class="space-y-4">
          <section class="studio-panel space-y-3 p-4">
            <h2 class="text-sm font-semibold">{{ t('providerDetail.configOwnership') }}</h2>
            <div
              class="flex items-start gap-2 rounded-lg border p-3 text-xs"
              :class="
                isConfigProvider
                  ? 'border-warning/40 bg-warning/10 text-warning'
                  : 'border-border/70 bg-muted/30 text-muted-foreground'
              "
            >
              <ShieldAlert class="mt-0.5 size-4 shrink-0" aria-hidden="true" />
              <span>
                {{
                  isConfigProvider
                    ? t('providerDetail.configOwnershipConfig')
                    : t('providerDetail.configOwnershipDb')
                }}
              </span>
            </div>

            <dl class="divide-y divide-border/50 text-xs">
              <div class="flex items-center justify-between gap-3 py-2">
                <dt class="text-muted-foreground">{{ t('providerDetail.desiredRevision') }}</dt>
                <dd class="font-mono">r{{ provider.runtime?.desired_revision ?? provider.revision }}</dd>
              </div>
              <div class="flex items-center justify-between gap-3 py-2">
                <dt class="text-muted-foreground">Source</dt>
                <dd class="font-mono">{{ provider.source || 'database' }}</dd>
              </div>
              <div class="flex items-center justify-between gap-3 py-2">
                <dt class="text-muted-foreground">Secret</dt>
                <dd class="font-mono">write-only</dd>
              </div>
            </dl>
          </section>

          <section class="studio-panel space-y-2 p-4">
            <h2 class="text-sm font-semibold">{{ t('providerDetail.schemaHintTitle') }}</h2>
            <p class="text-xs leading-relaxed text-muted-foreground">
              {{ t('providerDetail.secretHint') }}
            </p>
          </section>
        </div>
      </div>

      <!-- TAB 3: RUNTIME -->
      <div
        v-show="activeTab === 'runtime'"
        class="grid gap-4 lg:grid-cols-[minmax(0,1.15fr)_minmax(0,0.85fr)]"
      >
        <section class="studio-panel space-y-3 p-4">
          <div class="flex items-center justify-between gap-3 border-b border-border/60 pb-2.5">
            <div>
              <h2 class="text-sm font-semibold">{{ t('providerDetail.runtimeLifecycle') }}</h2>
              <p class="mt-0.5 text-xs text-muted-foreground">{{ t('providerDetail.runtimeNote') }}</p>
            </div>
            <Button
              type="button"
              size="sm"
              :disabled="busy || !enabled || !provider.runtime?.can_prepare"
              @click="prepare"
            >
              <Play class="size-3.5" aria-hidden="true" />
              {{ t('providerDetail.prepare') }}
            </Button>
          </div>

          <dl class="grid gap-2.5 sm:grid-cols-2 text-xs">
            <div class="rounded-lg border border-border/60 bg-muted/20 p-2.5">
              <dt class="text-muted-foreground">{{ t('providerDetail.desiredState') }}</dt>
              <dd class="mt-1 font-mono font-semibold">{{ runtimeState }}</dd>
            </div>
            <div class="rounded-lg border border-border/60 bg-muted/20 p-2.5">
              <dt class="text-muted-foreground">{{ t('providerDetail.readyRevisions') }}</dt>
              <dd class="mt-1 font-mono">
                {{
                  provider.runtime?.ready_revisions?.length
                    ? provider.runtime.ready_revisions.map((item) => `r${item}`).join(', ')
                    : '—'
                }}
              </dd>
            </div>
            <div class="rounded-lg border border-border/60 bg-muted/20 p-2.5">
              <dt class="text-muted-foreground">{{ t('providerDetail.runtimeMatch') }}</dt>
              <dd class="mt-1 font-semibold">
                {{ isRuntimeReady ? t('providerDetail.matches') : t('providerDetail.notReady') }}
              </dd>
            </div>
            <div class="rounded-lg border border-border/60 bg-muted/20 p-2.5">
              <dt class="text-muted-foreground">{{ t('providerDetail.failureCode') }}</dt>
              <dd class="mt-1 font-mono text-danger">
                {{ provider.runtime?.failure_code || '—' }}
              </dd>
            </div>
          </dl>

          <p v-if="provider.requires_restart" class="text-xs text-warning">
            {{ t('providerDetail.restartRequired') }}
          </p>
          <p v-if="!enabled" class="text-xs text-muted-foreground">
            {{ t('providerDetail.disabledPrepare') }}
          </p>
          <p v-else-if="!provider.runtime?.can_prepare" class="text-xs text-muted-foreground">
            {{ t('providerDetail.prepareUnavailable') }}
          </p>
        </section>

        <div class="space-y-4">
          <section class="studio-panel space-y-2.5 p-4">
            <h2 class="text-sm font-semibold">{{ t('providerDetail.lifecycleStates') }}</h2>
            <div class="divide-y divide-border/50 text-xs">
              <div
                v-for="item in lifecycleList"
                :key="item.id"
                class="flex items-center justify-between gap-2 py-2"
              >
                <span class="inline-flex items-center gap-2 font-mono">
                  <Activity class="size-3.5 text-muted-foreground" aria-hidden="true" />
                  {{ item.id }}
                </span>
                <span
                  v-if="runtimeState === item.id"
                  class="rounded-full border border-primary/40 bg-primary/12 px-2 py-0.5 text-[10px] font-semibold text-primary"
                >
                  {{ t('providerDetail.currentState') }}
                </span>
                <span v-else class="text-[11px] text-muted-foreground">{{ item.label }}</span>
              </div>
            </div>
          </section>
        </div>
      </div>

      <!-- TAB 4: DIAGNOSTICS / TEST -->
      <div
        v-show="activeTab === 'test'"
        class="grid gap-4 lg:grid-cols-[minmax(0,1.2fr)_minmax(0,0.8fr)]"
      >
        <section class="studio-panel space-y-3 p-4">
          <div class="flex items-center justify-between gap-2 border-b border-border/60 pb-2.5">
            <div>
              <h2 class="text-sm font-semibold">{{ t('providers.testTitle') }}</h2>
              <p class="mt-0.5 text-xs text-muted-foreground">{{ t('providerDetail.testHint') }}</p>
            </div>
            <span class="rounded-full border border-primary/30 bg-primary/10 px-2.5 py-0.5 text-[11px] font-semibold text-primary">
              {{ providerTypeLabel(provider.type) }} TEST
            </span>
          </div>

          <ProviderTestPanel
            :type="provider.type"
            :saved-key="provider.key"
            :revision="provider.revision"
            :adapter="provider.adapter"
            :model="model"
            :capabilities="adapterDescriptor?.capabilities"
          />
        </section>

        <section class="studio-panel space-y-3 p-4">
          <h2 class="border-b border-border/60 pb-2.5 text-sm font-semibold">
            {{ t('providerDetail.testContext') }}
          </h2>
          <dl class="divide-y divide-border/50 text-xs">
            <div class="flex items-center justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('providers.name') }}</dt>
              <dd class="font-medium">{{ provider.name }}</dd>
            </div>
            <div class="flex items-center justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('providers.adapter') }}</dt>
              <dd class="font-mono">{{ provider.adapter }}</dd>
            </div>
            <div class="flex items-center justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('providers.model') }}</dt>
              <dd class="font-mono">{{ model || '—' }}</dd>
            </div>
            <div class="flex items-center justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('providerDetail.desiredRevision') }}</dt>
              <dd class="font-mono">r{{ provider.runtime?.desired_revision ?? provider.revision }}</dd>
            </div>
            <div class="flex items-center justify-between gap-3 py-2">
              <dt class="text-muted-foreground">Input kind</dt>
              <dd class="font-mono">
                {{
                  provider.type === 'asr'
                    ? 'audio/wav'
                    : provider.type === 'vad'
                      ? 'PCM sample'
                      : 'text'
                }}
              </dd>
            </div>
          </dl>
        </section>
      </div>

      <!-- TAB 5: TEMPLATE BINDINGS -->
      <div
        v-show="activeTab === 'bindings'"
        class="grid gap-4 lg:grid-cols-[minmax(0,1.2fr)_minmax(0,0.8fr)]"
      >
        <section class="studio-panel space-y-3 p-4">
          <div class="flex items-start justify-between gap-3 border-b border-border/60 pb-2.5">
            <div>
              <h2 class="text-sm font-semibold">{{ t('providers.templateUsage') }}</h2>
              <p class="mt-0.5 text-xs text-muted-foreground">
                {{ t('providerDetail.bindingDescription') }}
              </p>
            </div>
            <span class="rounded-full border border-border/70 bg-muted/40 px-2.5 py-0.5 text-xs font-medium tabular-nums text-muted-foreground">
              {{ t('providers.templatesShort', { count: bindingTotal }) }}
            </span>
          </div>

          <p v-if="!bindings.length" class="text-xs text-muted-foreground">
            {{ t('providers.usageEmpty') }}
          </p>

          <ul v-else class="divide-y divide-border/60 text-xs">
            <li v-for="template in bindings" :key="template.key" class="py-2.5">
              <RouterLink
                :to="`/templates/${template.key}`"
                class="flex items-center justify-between gap-3 rounded-md p-1 transition-colors hover:bg-accent"
              >
                <span class="flex min-w-0 items-center gap-2">
                  <LayoutTemplate class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                  <span class="truncate font-medium">{{ template.name }}</span>
                </span>
                <span class="shrink-0 font-mono text-xs text-muted-foreground">{{ template.key }}</span>
              </RouterLink>
            </li>
          </ul>

          <Button
            v-if="bindings.length < bindingTotal"
            type="button"
            variant="outline"
            size="sm"
            :disabled="loadingBindings"
            @click="loadMoreBindings"
          >
            {{ loadingBindings ? t('common.loading') : t('providerDetail.loadMore') }}
          </Button>
        </section>

        <section class="studio-panel space-y-3 p-4">
          <h2 class="border-b border-border/60 pb-2.5 text-sm font-semibold">
            {{ t('providerDetail.dependencyGuard') }}
          </h2>
          <div
            class="flex items-start gap-2 rounded-lg border p-3 text-xs"
            :class="
              bindingTotal > 0 || isConfigProvider
                ? 'border-warning/40 bg-warning/10 text-warning'
                : 'border-border/70 bg-muted/30 text-muted-foreground'
            "
          >
            <ShieldAlert class="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            <span>
              {{
                bindingTotal > 0
                  ? t('providerDetail.deleteGuard')
                  : isConfigProvider
                    ? t('providerDetail.configOwnershipConfig')
                    : t('providerDelete.unused')
              }}
            </span>
          </div>

          <dl class="divide-y divide-border/50 text-xs">
            <div class="flex items-center justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('providers.templateUsage') }}</dt>
              <dd class="font-mono">{{ bindingTotal }}</dd>
            </div>
            <div class="flex items-center justify-between gap-3 py-2">
              <dt class="text-muted-foreground">{{ t('providerDetail.deletionAllowed') }}</dt>
              <dd class="font-mono">{{ !bindingTotal && !isConfigProvider ? 'Yes' : 'No' }}</dd>
            </div>
          </dl>
        </section>
      </div>
    </template>

    <ProviderFormModal
      v-if="editableProvider && !isConfigProvider"
      v-model="editOpen"
      :provider="editableProvider"
      @submit="save"
    />

    <ConfirmDialog
      v-model="deleteOpen"
      :title="t('providerDelete.title', { name: provider?.name ?? '' })"
      :description="t('providerDelete.description')"
      :confirm-label="t('providerDelete.submit')"
      @confirm="remove"
    >
      <p class="text-xs text-muted-foreground">{{ t('providerDelete.unused') }}</p>
    </ConfirmDialog>
  </div>
</template>

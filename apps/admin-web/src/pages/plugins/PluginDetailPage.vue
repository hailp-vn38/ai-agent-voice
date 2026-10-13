<script setup lang="ts">
import { Archive, Code2, Cpu, FlaskConical, FileCode2, LockKeyhole, RefreshCw, ShieldAlert, Trash2 } from '@lucide/vue'
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { agentPluginsApi } from '@/api/agent-plugins'
import { formatApiError, isApiError } from '@/api/errors'
import type { InstalledPlugin, PluginStatus } from '@/api/types/agent-jobs'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

type DetailTab = 'overview' | 'code' | 'permissions' | 'logs'
type GuardedAction = 'approve' | 'uninstall'
const route = useRoute()
const router = useRouter()
const { t } = useI18n()
const pluginId = computed(() => String(route.params.id ?? ''))
const plugin = ref<InstalledPlugin | null>(null)
const tab = ref<DetailTab>('overview')
const loading = ref(true)
const busy = ref(false)
const error = ref('')
const notice = ref('')
const confirmOpen = ref(false)
const pendingAction = ref<GuardedAction>('approve')
let controller: AbortController | undefined

const statusVariant = (status: PluginStatus): 'success' | 'danger' | 'secondary' =>
  status === 'ready' ? 'success' : status === 'error' ? 'danger' : 'secondary'
const statusLabel = (status: PluginStatus) =>
  t(('jobUi.pluginStatus.' + status) as 'jobUi.pluginStatus.ready')
const tabs = computed(() => [
  { key: 'overview' as DetailTab, label: t('jobUi.detailOverview') },
  { key: 'code' as DetailTab, label: t('jobUi.detailCode') },
  { key: 'permissions' as DetailTab, label: t('jobUi.declaredPermissions') },
  { key: 'logs' as DetailTab, label: t('jobUi.detailLogs') },
])
const permissionsApproved = computed(() => {
  const item = plugin.value
  if (!item) return false
  return item.permissions_approved === true
})

async function load() {
  controller?.abort()
  const pending = new AbortController()
  controller = pending
  loading.value = true
  error.value = ''
  try {
    const loaded = await agentPluginsApi.get(pluginId.value, pending.signal)
    if (!pending.signal.aborted) plugin.value = loaded
  } catch (cause) {
    if (!pending.signal.aborted) {
      plugin.value = null
      error.value = formatApiError(cause)
    }
  } finally {
    if (controller === pending) loading.value = false
  }
}

function confirmAction(type: GuardedAction) {
  pendingAction.value = type
  confirmOpen.value = true
}
async function action(type: 'test' | 'activate' | 'disable' | GuardedAction) {
  const item = plugin.value
  if (!item || busy.value) return
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    if (type === 'test') {
      const checked = await agentPluginsApi.test(item.id, item.revision)
      notice.value = checked.message || t(checked.ok ? 'jobUi.testPassed' : 'jobUi.testFailed')
    } else if (type === 'uninstall') {
      await agentPluginsApi.uninstall(item.id, item.revision)
      await router.push({ name: 'plugins' })
      return
    } else if (type === 'approve') {
      plugin.value = await agentPluginsApi.approvePermissions(item.id, item.revision, {
        network_hosts: item.permissions?.network_hosts ?? [],
        scratch_storage: item.permissions?.scratch_storage ?? false,
      })
      notice.value = t('jobUi.saved')
    } else {
      plugin.value = type === 'activate'
        ? await agentPluginsApi.activate(item.id, item.revision)
        : await agentPluginsApi.disable(item.id, item.revision)
      notice.value = t('jobUi.saved')
    }
  } catch (cause) {
    error.value = formatApiError(cause)
    if (isApiError(cause) && cause.status === 409) await load()
  } finally {
    busy.value = false
  }
}
function confirmedAction() {
  void action(pendingAction.value)
}
watch(pluginId, () => {
  tab.value = 'overview'
  plugin.value = null
  void load()
}, { immediate: true })
onBeforeUnmount(() => controller?.abort())
</script>

<template>
  <section class="space-y-5">
    <DetailHeader :title="plugin?.name || pluginId" :back-label="t('jobUi.plugins')" @back="router.push({ name: 'plugins' })">
      <template #icon><div class="grid size-10 shrink-0 place-items-center rounded-xl bg-studio-violet/10 text-studio-violet"><Archive class="size-5" /></div></template>
      <template #details>
        <span class="break-all font-mono">{{ pluginId }}</span>
        <span v-if="plugin"> · v{{ plugin.version }} · {{ plugin.runtime.language }}</span>
      </template>
      <template #actions>
        <Badge v-if="plugin" :variant="statusVariant(plugin.status)">{{ statusLabel(plugin.status) }}</Badge>
        <Button variant="outline" :disabled="busy || loading" @click="load"><RefreshCw class="size-4" />{{ t('jobUi.refresh') }}</Button>
        <Button v-if="plugin" variant="outline" :disabled="busy" @click="action('test')"><FlaskConical class="size-4" />{{ t('jobUi.test') }}</Button>
        <Button v-if="plugin && plugin.status === 'ready'" variant="outline" :disabled="busy" @click="action('disable')">{{ t('jobUi.disable') }}</Button>
        <Button v-if="plugin && plugin.status !== 'ready'" :disabled="busy || !permissionsApproved" @click="action('activate')">
          {{ plugin.status === 'error' ? t('jobUi.reset') : t('jobUi.activate') }}
        </Button>
      </template>
    </DetailHeader>

    <div v-if="error" role="alert" class="rounded-xl border border-danger/30 bg-danger/5 p-4 text-sm text-danger">{{ error }}</div>
    <p v-if="notice" role="status" class="rounded-xl border border-success/30 bg-success/5 p-3 text-sm text-success-foreground">{{ notice }}</p>
    <div v-if="loading" class="studio-panel p-8 text-center text-sm text-muted-foreground">{{ t('common.loading') }}</div>

    <template v-if="plugin && !loading">
      <p class="text-sm text-muted-foreground">{{ plugin.description }}</p>
      <div v-if="plugin.last_error_code" class="rounded-xl border border-danger/30 bg-danger/5 p-4 text-sm">
        <div class="flex items-center gap-2 font-semibold text-danger"><ShieldAlert class="size-4" />{{ t('jobUi.lastError') }}</div>
        <p class="mt-2 break-all font-mono text-xs text-danger">{{ plugin.last_error_code }}</p>
      </div>

      <div class="grid gap-4 xl:grid-cols-[minmax(0,1fr)_320px]">
        <div class="min-w-0 space-y-4">
          <div role="tablist" :aria-label="t('jobUi.pluginDetailTabs')" class="flex flex-wrap gap-1 border-b">
            <button v-for="item in tabs" :key="item.key" type="button" role="tab" :aria-selected="tab === item.key"
              class="border-b-2 px-4 py-3 text-sm font-medium" :class="tab === item.key ? 'border-studio-violet text-foreground' : 'border-transparent text-muted-foreground hover:text-foreground'"
              @click="tab = item.key">{{ item.label }}</button>
          </div>
          <section v-if="tab === 'overview'" class="studio-panel p-5">
            <h2 class="mb-4 flex items-center gap-2 font-semibold"><FileCode2 class="size-4 text-studio-violet" />{{ t('jobUi.operations') }}</h2>
            <p v-if="!plugin.operations.length" class="text-sm text-muted-foreground">{{ t('jobUi.noOperations') }}</p>
            <div v-for="operation in plugin.operations" :key="operation.name" class="flex items-start justify-between gap-3 border-t py-3 text-sm">
              <div class="min-w-0">
                <code class="break-all font-medium">{{ operation.name }}</code>
                <p v-if="operation.description" class="mt-1 text-muted-foreground">{{ operation.description }}</p>
              </div>
              <Badge variant="outline">{{ operation.replay || '—' }}</Badge>
            </div>
          </section>
          <section v-else-if="tab === 'code'" class="studio-panel space-y-4 p-5">
            <h2 class="flex items-center gap-2 font-semibold"><Code2 class="size-4 text-studio-violet" />{{ t('jobUi.detailCode') }}</h2>
            <div class="grid gap-3 sm:grid-cols-2">
              <div class="rounded-xl border p-4">
                <p class="text-xs font-medium text-muted-foreground">{{ t('jobUi.packageDigest') }}</p>
                <code class="mt-2 block break-all text-xs">{{ plugin.digest || '—' }}</code>
              </div>
              <div class="rounded-xl border p-4">
                <p class="text-xs font-medium text-muted-foreground">{{ t('jobUi.imageDigest') }}</p>
                <code class="mt-2 block break-all text-xs">{{ plugin.image_digest || '—' }}</code>
              </div>
            </div>
            <p class="text-sm leading-relaxed text-muted-foreground">{{ t('jobUi.sourceUnavailable') }}</p>
          </section>
          <section v-else-if="tab === 'permissions'" class="studio-panel space-y-4 p-5">
            <div class="flex flex-wrap items-center justify-between gap-2">
              <h2 class="font-semibold">{{ t('jobUi.declaredPermissions') }}</h2>
              <Badge :variant="permissionsApproved ? 'success' : 'secondary'">
                {{ permissionsApproved ? t('jobUi.permissionsApproved') : t('jobUi.permissionsPending') }}
              </Badge>
            </div>
            <div class="divide-y text-sm">
              <div class="flex flex-wrap justify-between gap-2 py-3">
                <span class="text-muted-foreground">{{ t('jobUi.network') }}</span>
                <span class="max-w-full break-all font-mono">{{ plugin.permissions?.network_hosts.join(', ') || '—' }}</span>
              </div>
              <div class="flex justify-between gap-2 py-3">
                <span class="text-muted-foreground">{{ t('jobUi.scratch') }}</span>
                <span>{{ plugin.permissions?.scratch_storage ? t('jobUi.yes') : t('jobUi.no') }}</span>
              </div>
              <div class="flex flex-wrap justify-between gap-2 py-3">
                <span class="text-muted-foreground">{{ t('jobUi.approvedNetwork') }}</span>
                <span class="break-all font-mono">{{ plugin.approved_permissions?.network_hosts.join(', ') || '—' }}</span>
              </div>
            </div>
            <p class="text-xs text-muted-foreground">{{ t('jobUi.permissionsHint') }}</p>
            <Button v-if="!permissionsApproved" variant="outline" :disabled="busy" @click="confirmAction('approve')"><LockKeyhole class="size-4" />{{ t('jobUi.approve') }}</Button>
          </section>
          <section v-else class="studio-panel space-y-3 p-5">
            <h2 class="font-semibold">{{ t('jobUi.detailLogs') }}</h2>
            <p v-if="plugin.last_error_code" class="break-all rounded-lg bg-muted p-3 font-mono text-xs">{{ plugin.last_error_code }}</p>
            <p class="text-sm text-muted-foreground">{{ t('jobUi.pluginLogsUnavailable') }}</p>
          </section>
          <section class="studio-panel p-5">
            <div class="flex items-center justify-between gap-3">
              <div>
                <h2 class="font-semibold">{{ t('jobUi.uninstall') }}</h2>
                <p class="mt-1 text-sm text-muted-foreground">{{ t('jobUi.uninstallConfirm') }}</p>
              </div>
              <Button variant="outline" :disabled="busy" @click="confirmAction('uninstall')"><Trash2 class="size-4" />{{ t('jobUi.uninstall') }}</Button>
            </div>
          </section>
        </div>
        <aside class="min-w-0 space-y-4">
          <section class="studio-panel p-5">
            <div class="flex items-center justify-between gap-2"><h2 class="font-semibold">{{ t('jobUi.pluginHealth') }}</h2><Badge :variant="statusVariant(plugin.status)">{{ statusLabel(plugin.status) }}</Badge></div>
            <p class="mt-4 text-sm" :class="plugin.last_error_code ? 'text-danger' : 'text-muted-foreground'">
              {{ plugin.last_error_code || (plugin.status === 'ready' ? t('jobUi.pluginReadyHint') : t('jobUi.pluginInactiveHint')) }}
            </p>
            <div class="mt-4 divide-y text-sm">
              <div class="flex justify-between gap-3 py-3"><span class="text-muted-foreground">{{ t('jobUi.runtime') }}</span><span class="text-right">{{ plugin.runtime.language }} / {{ plugin.runtime.kind }}</span></div>
              <div class="flex justify-between gap-3 py-3"><span class="text-muted-foreground">{{ t('jobUi.version') }}</span><code>{{ plugin.version }}</code></div>
              <div class="flex justify-between gap-3 py-3"><span class="text-muted-foreground">{{ t('jobUi.status') }}</span><span>{{ statusLabel(plugin.status) }}</span></div>
            </div>
          </section>
          <section class="studio-panel p-5">
            <h2 class="flex items-center gap-2 font-semibold"><Cpu class="size-4 text-studio-violet" />{{ t('jobUi.resourceLimits') }}</h2>
            <p class="mt-3 text-sm text-muted-foreground">{{ t('jobUi.pluginResourcesHint') }}</p>
          </section>
        </aside>
      </div>
    </template>
    <ConfirmDialog v-model="confirmOpen"
      :title="pendingAction === 'approve' ? t('jobUi.approve') : t('jobUi.uninstall')"
      :confirm-label="pendingAction === 'approve' ? t('jobUi.approve') : t('jobUi.uninstall')"
      @confirm="confirmedAction">
      <p>{{ pendingAction === 'approve' ? t('jobUi.approveConfirm') : t('jobUi.uninstallConfirm') }}</p>
      <p v-if="pendingAction === 'approve'" class="mt-2 break-all font-mono text-xs text-muted-foreground">
        {{ plugin?.permissions?.network_hosts.join(', ') || '—' }} · {{ plugin?.digest }}
      </p>
    </ConfirmDialog>
  </section>
</template>

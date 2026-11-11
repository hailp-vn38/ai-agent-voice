<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import type { SyncBackend, SyncConfiguration, SyncSettingsInput, SyncStatus } from './sync'
import { syncErrorMessage, syncStatusLabel } from './sync'
import { useI18n } from './integration'
import KnowledgeSyncBadge from './KnowledgeSyncBadge.vue'

const props = defineProps<{ backend?: SyncBackend }>()
const emit = defineEmits<{
  close: []
  openConflict: [path: { scope: 'raw' | 'wiki'; relative: string }]
}>()
const { t } = useI18n()
const status = ref<SyncStatus>({ type: 'disabled', reason: 'not_configured' })
const loading = ref(true)
const publicGitHubRemote = ref(false)
const busy = ref(false)
const failure = ref<unknown>(null)
const locale = computed(() => document.documentElement.lang === 'vi' ? 'vi' : 'en')
const statusLabel = computed(() => syncStatusLabel(status.value, locale.value))
const errorLabel = computed(() => failure.value ? syncErrorMessage(failure.value, locale.value) : '')
const settings = reactive<SyncConfiguration>({ remote_url: '', branch: 'main', auto: false, interval_min: 5 })
let stopStatus: (() => void) | undefined

async function refresh() {
  if (!props.backend) return
  loading.value = true
  try {
    const [nextStatus, configuration] = await Promise.all([
      props.backend.status(),
      props.backend.configuration?.(),
    ])
    status.value = nextStatus
    if (configuration) {
      Object.assign(settings, {
        remote_url: configuration.remote_url,
        branch: configuration.branch,
        auto: configuration.auto,
        interval_min: configuration.interval_min,
      })
      publicGitHubRemote.value = configuration.public_github_remote ?? false
    }
  } finally { loading.value = false }
}

async function syncNow() {
  if (!props.backend) return
  busy.value = true
  failure.value = null
  try {
    await props.backend.syncNow()
    await refresh()
  } catch (cause) {
    failure.value = cause
  } finally { busy.value = false }
}

async function save() {
  if (!props.backend?.configure) return
  busy.value = true
  failure.value = null
  try {
    const input: SyncSettingsInput = { ...settings }
    status.value = await props.backend.configure(input)
    await refresh()
  } catch (cause) {
    failure.value = cause
  } finally { busy.value = false }
}

async function disable() {
  if (!props.backend?.disable) return
  busy.value = true
  failure.value = null
  try {
    await props.backend.disable()
    await refresh()
  } catch (cause) {
    failure.value = cause
  } finally { busy.value = false }
}

onMounted(() => {
  if (!props.backend) return
  stopStatus = props.backend.onStatus(next => { status.value = next })
  void refresh().catch(cause => { failure.value = cause })
})
onBeforeUnmount(() => stopStatus?.())
</script>

<template>
  <section v-if="backend" class="knowledge-sync-panel" data-testid="knowledge-sync-panel" aria-labelledby="knowledge-sync-title">
    <header class="knowledge-sync-panel__header">
      <h2 id="knowledge-sync-title">{{ t('knowledgeSync.title') }}</h2>
      <button type="button" :aria-label="t('knowledgeSync.close')" @click="emit('close')">×</button>
    </header>
    <div v-if="loading" class="knowledge-sync-panel__loading card-shimmer" role="status" aria-busy="true">
      <span>{{ t('knowledgeSync.loading') }}</span>
    </div>
    <KnowledgeSyncBadge v-else :status="status" />
    <p v-if="errorLabel" role="alert" class="knowledge-sync-panel__error">{{ errorLabel }}</p>
    <p v-if="!loading && publicGitHubRemote" class="knowledge-sync-panel__warning" role="status">{{ t('knowledgeSync.publicRemoteWarning') }}</p>

    <div v-if="loading" class="knowledge-sync-panel__details-loading card-shimmer" role="status" aria-busy="true">
      <span>{{ t('knowledgeSync.loading') }}</span>
    </div>
    <form v-else-if="backend.configurable" data-testid="sync-settings-form" @submit.prevent="save">
      <label>
        {{ t('knowledgeSync.remote') }}
        <input v-model.trim="settings.remote_url" type="text" inputmode="url" autocomplete="url" required>
      </label>
      <label>
        {{ t('knowledgeSync.branch') }}
        <input v-model.trim="settings.branch" type="text" required>
      </label>
      <label class="knowledge-sync-panel__toggle">
        <input v-model="settings.auto" type="checkbox">
        {{ t('knowledgeSync.auto') }}
      </label>
      <label>
        {{ t('knowledgeSync.interval') }}
        <input v-model.number="settings.interval_min" type="number" min="1" max="1440" required>
      </label>
      <div class="knowledge-sync-panel__actions">
        <button type="submit" :disabled="busy">{{ t('knowledgeSync.save') }}</button>
        <button type="button" :disabled="busy" @click="syncNow">{{ t('knowledgeSync.syncNow') }}</button>
        <button type="button" :disabled="busy" @click="disable">{{ t('knowledgeSync.disable') }}</button>
      </div>
    </form>
    <div v-else class="knowledge-sync-panel__readonly">
      <dl class="knowledge-sync-panel__configuration">
        <dt>{{ t('knowledgeSync.remote') }}</dt><dd>{{ settings.remote_url || '—' }}</dd>
        <dt>{{ t('knowledgeSync.branch') }}</dt><dd>{{ settings.branch }}</dd>
        <dt>{{ t('knowledgeSync.auto') }}</dt><dd>{{ settings.auto ? t('knowledgeSync.autoOn') : t('knowledgeSync.autoOff') }}</dd>
        <dt>{{ t('knowledgeSync.interval') }}</dt><dd>{{ settings.interval_min }}</dd>
      </dl>
      <p>{{ t('knowledgeSync.configInToml') }}</p>
      <button type="button" :disabled="busy" @click="syncNow">{{ t('knowledgeSync.syncNow') }}</button>
    </div>

    <ul v-if="status.type === 'conflicted' && status.copies.length" class="knowledge-sync-panel__conflicts">
      <li v-for="copy in status.copies" :key="`${copy.scope}:${copy.relative}`">
        <button type="button" @click="emit('openConflict', copy)">{{ copy.scope }}/{{ copy.relative }}</button>
      </li>
    </ul>
  </section>
</template>

<style scoped>
.knowledge-sync-panel { display: grid; gap: 1rem; min-width: min(28rem, calc(100vw - 3rem)); color: var(--foreground, #20242a); }
.knowledge-sync-panel__header, .knowledge-sync-panel__actions { display: flex; align-items: center; justify-content: space-between; gap: .6rem; }
.knowledge-sync-panel__header h2 { margin: 0; font-size: 1.15rem; }
.knowledge-sync-panel form { display: grid; gap: .8rem; }
.knowledge-sync-panel label { display: grid; gap: .35rem; font-size: .9rem; }
.knowledge-sync-panel input:not([type=checkbox]) { width: 100%; min-height: 2.4rem; padding: .4rem .6rem; }
.knowledge-sync-panel__toggle { display: flex !important; align-items: center; }
.knowledge-sync-panel__actions { justify-content: flex-start; flex-wrap: wrap; }
.knowledge-sync-panel button { cursor: pointer; }
.knowledge-sync-panel button:disabled { cursor: wait; opacity: .6; }
.knowledge-sync-panel__error { color: #b42318; }
.knowledge-sync-panel__warning { padding: .65rem; border-inline-start: 3px solid #d97706; background: color-mix(in srgb, #d97706 10%, transparent); }
.knowledge-sync-panel__loading { min-height: 2.6rem; border-radius: .35rem; }
.knowledge-sync-panel__details-loading { min-height: 10rem; border-radius: .35rem; }
.knowledge-sync-panel__conflicts { margin: 0; padding-inline-start: 1.2rem; }
.knowledge-sync-panel__conflicts button { border: 0; padding: .25rem; color: inherit; background: transparent; text-decoration: underline; }
.knowledge-sync-panel__configuration { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: .4rem .8rem; margin: 0; overflow-wrap: anywhere; }
.knowledge-sync-panel__configuration dt { font-weight: 600; }
.knowledge-sync-panel__configuration dd { margin: 0; }
</style>

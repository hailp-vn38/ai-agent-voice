<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { agentPluginsApi } from '@/api/agent-plugins'
import { formatApiError, isApiError } from '@/api/errors'
import type { PluginOperationContract, PluginValueSettings, PluginArtifactBinding } from '@/api/types/agent-jobs'
import { Button } from '@/components/ui/button'

const props = defineProps<{
  pluginKey: string
  operations: string[]
  enabled: boolean
  mode: 'settings' | 'functions' | 'runs'
}>()
const router = useRouter()
const busy = ref(false)
const loading = ref(false)
const error = ref('')
const notice = ref('')
const settings = ref<PluginValueSettings | null>(null)
const edited = ref<Record<string, unknown>>({})
const selected = ref('')
const argumentsJson = ref('{}')
const bindingsJson = ref('[]')
let lastRunAttempt: { signature: string; key: string } | null = null
const contract = ref<PluginOperationContract | null>(null)
const runs = ref<Array<{id:string;status:string;created_at:string}>>([])
const runsPage = ref(1)
const runsHasMore = ref(false)
let controller: AbortController | null = null

const fields = computed(() => Object.entries(settings.value?.schema.properties ?? {})
  .map(([name, definition]) => ({ name, ...definition,
    required: settings.value?.schema.required?.includes(name) ?? false })))

async function loadRuns(page = 1) {
  loading.value = true
  error.value = ''
  try {
    const result = await agentPluginsApi.runs(props.pluginKey, undefined, page)
    runs.value = result.items
    runsPage.value = result.page
    runsHasMore.value = result.page * result.page_size < result.total
  } catch (cause) { error.value = formatApiError(cause) }
  finally { loading.value = false }
}
async function loadSettings() {
  controller?.abort()
  const pending = new AbortController()
  controller = pending
  loading.value = true
  error.value = ''
  try {
    const loaded = await agentPluginsApi.settings(props.pluginKey, pending.signal)
    if (pending.signal.aborted) return
    settings.value = loaded
    edited.value = { ...loaded.values }
  } catch (cause) {
    if (!pending.signal.aborted) error.value = formatApiError(cause)
  } finally {
    if (controller === pending) loading.value = false
  }
}
async function saveSettings() {
  if (!settings.value || busy.value) return
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    const outgoing = { ...edited.value }
    for (const field of fields.value) {
      if (field['x-secret'] && outgoing[field.name] === '') delete outgoing[field.name]
    }
    const updated = await agentPluginsApi.updateSettings(props.pluginKey, outgoing, settings.value.revision)
    settings.value = updated
    edited.value = { ...updated.values }
    notice.value = 'Settings saved.'
  } catch (cause) {
    error.value = formatApiError(cause)
    if (isApiError(cause) && cause.status === 409) await loadSettings()
  } finally { busy.value = false }
}
async function loadOperation() {
  contract.value = null
  error.value = ''
  if (!selected.value) return
  loading.value = true
  try {
    contract.value = await agentPluginsApi.operation(props.pluginKey, selected.value)
  } catch (cause) { error.value = formatApiError(cause) }
  finally { loading.value = false }
}
async function runOperation() {
  if (busy.value || !selected.value || !props.enabled) return
  let argumentsValue: unknown
  let artifactBindings: PluginArtifactBinding[] = []
  try {
    argumentsValue = JSON.parse(argumentsJson.value)
    const decoded: unknown = JSON.parse(bindingsJson.value)
    if (!Array.isArray(decoded) || decoded.length > 4 || decoded.some(value =>
      !value || typeof value !== 'object' ||
      typeof value.argument_pointer !== 'string' ||
      typeof value.source_job_id !== 'string' ||
      typeof value.source_artifact_id !== 'string')) {
      throw new Error('Artifact bindings must be an array of up to four {argument_pointer, source_job_id, source_artifact_id} objects.')
    }
    artifactBindings = decoded as PluginArtifactBinding[]
    if (!argumentsValue || typeof argumentsValue !== 'object' || Array.isArray(argumentsValue)) {
      throw new Error('Arguments must be a JSON object.')
    }
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : 'Invalid JSON.'
    return
  }
  const signature = JSON.stringify({
    plugin: props.pluginKey, operation: selected.value,
    arguments: argumentsValue, artifactBindings,
  })
  if (!lastRunAttempt || lastRunAttempt.signature !== signature) {
    lastRunAttempt = {
      signature,
      key: globalThis.crypto?.randomUUID?.() ??
        `${Date.now()}-${Math.random().toString(36).slice(2)}`,
    }
  }
  busy.value = true
  error.value = ''
  try {
    const result = await agentPluginsApi.run(
      props.pluginKey, selected.value, argumentsValue as Record<string, unknown>,
      artifactBindings, lastRunAttempt.key,
    )
    await router.push({ name: 'job-detail', params: { jobId: result.job_id } })
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
function clearSecret(name: string) {
  // Null is the API's explicit deletion sentinel; omission keeps the old value.
  edited.value = { ...edited.value, [name]: null }
}
function keepSecret(name: string) {
  const next = { ...edited.value }
  delete next[name]
  edited.value = next
}
function changeValue(name: string, type: string, event: Event) {
  const target = event.target as HTMLInputElement
  edited.value = {
    ...edited.value,
    [name]: type === 'boolean' ? target.checked :
      type === 'integer' || type === 'number' ? Number(target.value) : target.value,
  }
}
watch(() => [props.pluginKey, props.mode], () => {
  notice.value = ''
  error.value = ''
  lastRunAttempt = null
  if (props.mode === 'settings') {
    void loadSettings()
  } else if (props.mode === 'runs') {
    controller?.abort()
    void loadRuns()
  } else {
    controller?.abort()
    selected.value = props.operations[0] ?? ''
    argumentsJson.value = '{}'
    bindingsJson.value = '[]'
    void loadOperation()
  }
}, { immediate: true })
onBeforeUnmount(() => controller?.abort())
</script>

<template>
  <section class="studio-panel space-y-4 p-5" :aria-busy="busy || loading">
    <p v-if="error" role="alert" class="break-words text-sm text-danger">{{ error }}</p>
    <p v-if="notice" role="status" class="text-sm text-muted-foreground">{{ notice }}</p>
    <p v-if="loading" class="text-sm text-muted-foreground">Loading...</p>
    <template v-if="mode === 'settings' && settings">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <h2 class="font-semibold">Plugin value settings</h2>
        <span class="text-xs text-muted-foreground">Revision {{ settings.revision }} · activation is separate</span>
      </div>
      <p v-if="settings.needs_reconfiguration" role="status"
        class="rounded-lg border border-warning/30 p-3 text-sm text-foreground">
        This plugin package or settings schema changed. Configure the new values before activation.
        Previous secret values will not be copied into this version.
      </p>
      <p v-if="fields.length === 0" class="text-sm text-muted-foreground">
        This plugin has no configurable values.
      </p>
      <div v-for="field in fields" :key="field.name" class="space-y-1">
        <label :for="'config-' + field.name" class="block text-sm font-medium">
          {{ field.name }}{{ field.required ? ' *' : '' }}
        </label>
        <p v-if="field.description" class="text-xs text-muted-foreground">{{ field.description }}</p>
        <label v-if="field.type === 'boolean'" class="inline-flex items-center gap-2 text-sm">
          <input :id="'config-' + field.name" type="checkbox" :checked="Boolean(edited[field.name])"
            :disabled="busy" @change="changeValue(field.name, field.type, $event)" />
          Enabled
        </label>
        <input v-else :id="'config-' + field.name" class="admin-input w-full" autocomplete="off"
          :type="field['x-secret'] ? 'password' : field.type === 'integer' || field.type === 'number' ? 'number' : 'text'"
          :step="field.type === 'integer' ? 1 : 'any'"
          :placeholder="field['x-secret'] && settings.secret_set[field.name] ? 'Configured · leave empty to retain' : ''"
          :value="String(edited[field.name] ?? '')" :disabled="busy"
          @input="changeValue(field.name, field.type, $event)" />
        <div v-if="field['x-secret']" class="space-y-2 text-xs text-muted-foreground">
          <p>Secret values are never returned by the server. Leave blank to keep existing.</p>
          <div v-if="settings.secret_set[field.name]" class="flex items-center gap-2">
            <Button v-if="edited[field.name] !== null" size="sm" variant="outline"
              :disabled="busy" @click="clearSecret(field.name)">Clear saved secret</Button>
            <template v-else>
              <span>Saved secret will be cleared after Save.</span>
              <Button size="sm" variant="outline" :disabled="busy"
                @click="keepSecret(field.name)">Undo clear</Button>
            </template>
          </div>
        </div>
      </div>
      <Button :disabled="busy || loading" @click="saveSettings">Save settings</Button>
    </template>
    <template v-else-if="mode === 'runs'">
      <h2 class="font-semibold">Plugin Runs</h2>
      <p class="text-sm text-muted-foreground">Manual executions recorded in the Job history.</p>
      <p v-if="!loading && runs.length === 0" class="text-sm text-muted-foreground">No runs yet.</p>
      <div v-for="run in runs" :key="run.id" class="flex items-center justify-between gap-3 rounded-lg border p-3 text-sm">
        <div class="min-w-0">
          <p class="font-mono text-xs">{{ run.id }}</p>
          <p class="text-xs text-muted-foreground">{{ run.created_at }} · {{ run.status }}</p>
        </div>
        <Button size="sm" variant="outline" @click="router.push({ name: 'job-detail', params: { jobId: run.id } })">Open Job</Button>
      </div>
      <div class="flex items-center gap-2">
        <Button size="sm" variant="outline" :disabled="loading || runsPage === 1" @click="loadRuns(runsPage - 1)">Previous</Button>
        <span class="text-xs text-muted-foreground">Page {{ runsPage }}</span>
        <Button size="sm" variant="outline" :disabled="loading || !runsHasMore" @click="loadRuns(runsPage + 1)">Next</Button>
      </div>
    </template>
    <template v-else-if="mode === 'functions'">
      <h2 class="font-semibold">Run declared operation</h2>
      <p class="text-sm text-muted-foreground">Runs as a durable, no-LLM Job. Only safe-read operations are supported.</p>
      <label class="block text-sm font-medium" for="plugin-operation">Operation</label>
      <select id="plugin-operation" v-model="selected" class="admin-input w-full" @change="loadOperation">
        <option v-for="name in operations" :key="name" :value="name">{{ name }}</option>
      </select>
      <div v-if="contract">
        <p class="text-sm text-muted-foreground">{{ contract.description }}</p>
        <p class="mt-3 text-xs font-semibold">Input JSON Schema</p>
        <pre class="max-h-48 overflow-auto rounded-lg border bg-muted/40 p-3 text-xs">{{ JSON.stringify(contract.input_schema, null, 2) }}</pre>
      </div>
      <label class="block text-sm font-medium" for="plugin-arguments">Arguments (JSON)</label>
      <textarea id="plugin-arguments" v-model="argumentsJson" rows="7" spellcheck="false"
        class="admin-input w-full font-mono text-sm" :disabled="busy || !enabled" />
      <label class="block text-sm font-medium" for="plugin-bindings">Input artifact bindings (JSON array)</label>
      <textarea id="plugin-bindings" v-model="bindingsJson" rows="4" spellcheck="false"
        class="admin-input w-full font-mono text-sm" :disabled="busy || !enabled" />
      <p class="text-xs text-muted-foreground">
        For Core document/artifact functions, reference a completed source Job's artifact with
        argument_pointer such as /artifact_id. The server imports checked bytes into the new Job.
      </p>
      <p v-if="!enabled" class="text-xs text-muted-foreground">Enable this plugin before running operations.</p>
      <Button :disabled="busy || !enabled || !selected || !contract" @click="runOperation">Run operation</Button>
    </template>
  </section>
</template>

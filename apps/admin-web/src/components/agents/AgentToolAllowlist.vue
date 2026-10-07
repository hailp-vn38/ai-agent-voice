<script setup lang="ts">
import { ref, watch } from 'vue'
import { jsonRequest, requestJson } from '@/api/client'
import { Button } from '@/components/ui/button'
const props = defineProps<{ agentId: string }>()
type Tool = { server_key: string; original_name: string; description: string; input_schema: unknown; source: unknown; fingerprint: string; observed_revision: number; observed_at: number; allowed: boolean; sensitive: boolean; revision: number }
const tools = ref<Tool[]>([])
const error = ref('')
const busy = ref(false)
const path = () => `/api/admin/agents/${encodeURIComponent(props.agentId)}/tool-allowlist`
async function load() {
  try { tools.value = (await requestJson<{ items: Tool[] }>(path())).items; error.value = '' }
  catch (e) { error.value = String(e) }
}
async function review(tool: Tool, allowed: boolean) {
  busy.value = true
  try {
    await requestJson(path(), jsonRequest('PUT', { server_key: tool.server_key, original_name: tool.original_name, observed_revision: tool.observed_revision, fingerprint: tool.fingerprint, allowed, sensitive: tool.sensitive }), { revision: tool.revision })
    await load()
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
watch(() => props.agentId, load, { immediate: true })
</script>
<template>
  <section aria-label="External tool review" class="space-y-3">
    <h2 class="font-semibold">External MCP tool review</h2>
    <p class="text-sm">Contracts observed during completed discovery. Observation does not prove current online presence. Sensitive tools are blocked. Grants apply to new connections.</p>
    <Button variant="outline" :disabled="busy" @click="load">Refresh observations</Button>
    <p v-if="error" role="alert">{{ error }}</p>
    <p v-if="!tools.length">No observed external tools.</p>
    <div v-for="tool in tools" :key="`${tool.server_key}/${tool.original_name}`" class="rounded border p-3 space-y-2">
      <strong>{{ tool.server_key }} / {{ tool.original_name }}</strong>
      <p>{{ tool.description }}</p>
      <p class="text-sm">Observed {{ new Date(tool.observed_at * 1000).toLocaleString() }}</p>
      <pre class="overflow-auto">{{ JSON.stringify(tool.source, null, 2) }}</pre>
      <details><summary>Input schema</summary><pre class="overflow-auto">{{ JSON.stringify(tool.input_schema, null, 2) }}</pre></details>
      <label><input v-model="tool.sensitive" type="checkbox" :disabled="busy"> Sensitive (blocked)</label>
      <Button :disabled="busy" @click="review(tool, !tool.allowed)">{{ tool.allowed ? 'Revoke' : 'Approve reviewed contract' }}</Button>
    </div>
  </section>
</template>

<script setup lang="ts">
import { ref, watch } from 'vue'
import { jsonRequest, requestJson } from '@/api/client'
import { Button } from '@/components/ui/button'
const props = defineProps<{ agentId: string }>()
type Tool = { device_id: string; original_name: string; description: string; input_schema: unknown; fingerprint: string; observed_revision: number; observed_at: number; allowed: boolean; sensitive: boolean; revision: number }
const tools = ref<Tool[]>([])
const error = ref('')
const busy = ref(false)
const path = () => `/api/admin/agents/${encodeURIComponent(props.agentId)}/device-tool-allowlist`
async function load() {
  try { tools.value = (await requestJson<{ items: Tool[] }>(path())).items; error.value = '' }
  catch (e) { error.value = String(e) }
}
async function review(tool: Tool, allowed: boolean) {
  busy.value = true
  try {
    await requestJson(path(), jsonRequest('PUT', { device_id: tool.device_id, original_name: tool.original_name, observed_revision: tool.observed_revision, fingerprint: tool.fingerprint, allowed, sensitive: tool.sensitive }), { revision: tool.revision })
    await load()
  } catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
watch(() => props.agentId, load, { immediate: true })
</script>
<template>
  <section aria-label="Device tool review" class="space-y-3">
    <h2 class="font-semibold">Device tool review</h2>
    <p class="text-sm">Contracts observed during completed Device discovery. Observation does not prove current online presence. Sensitive tools are blocked. Grants apply to new connections.</p>
    <Button variant="outline" :disabled="busy" @click="load">Refresh observations</Button>
    <p v-if="error" role="alert">{{ error }}</p>
    <p v-if="!tools.length">No observed Device tools.</p>
    <div v-for="tool in tools" :key="`${tool.device_id}/${tool.original_name}`" class="rounded border p-3 space-y-2">
      <strong>{{ tool.device_id }} / {{ tool.original_name }}</strong>
      <p>{{ tool.description }}</p>
      <p class="text-sm">Observed {{ new Date(tool.observed_at * 1000).toLocaleString() }}</p>
      <details><summary>Input schema</summary><pre class="overflow-auto">{{ JSON.stringify(tool.input_schema, null, 2) }}</pre></details>
      <label><input v-model="tool.sensitive" type="checkbox" :disabled="busy"> Sensitive (blocked)</label>
      <Button :disabled="busy" @click="review(tool, !tool.allowed)">{{ tool.allowed ? 'Revoke' : 'Approve reviewed contract' }}</Button>
    </div>
  </section>
</template>

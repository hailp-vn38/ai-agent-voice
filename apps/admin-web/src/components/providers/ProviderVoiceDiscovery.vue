<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'
import { providerAdaptersApi } from '@/api/provider-adapters'
import { providersApi } from '@/api/providers'
import { formatApiError } from '@/api/errors'
import type { ProviderTestDraft } from '@/api/types/providers'
import { Button } from '@/components/ui/button'

const props = defineProps<{ draft?: ProviderTestDraft; savedKey?: string }>()
const emit = defineEmits<{ discovered: [capabilities: Record<string, unknown>] }>()
const busy = ref(false)
const error = ref('')
let controller: AbortController | undefined
function cancel() { controller?.abort(); controller = undefined; busy.value = false; error.value = '' }
watch([() => props.draft?.config_json.base_url, () => props.draft?.api_key, () => props.draft?.saved_credential?.key, () => props.draft?.saved_credential?.expected_revision, () => props.savedKey], () => {
  cancel()
  emit('discovered', { voices: [] })
})
onBeforeUnmount(cancel)

async function discover() {
  cancel()
  const pending = new AbortController()
  controller = pending
  busy.value = true
  try {
    let draft = props.draft
    if (!draft && props.savedKey) {
      const saved = await providersApi.get(props.savedKey, pending.signal)
      if (saved.source === 'config') {
        const result = await providersApi.capabilities(saved.key, pending.signal)
        if (!pending.signal.aborted) emit('discovered', result.capabilities)
        return
      }
      draft = { type: 'tts', adapter: saved.adapter, config_json: JSON.parse(saved.config_json),
        ...(saved.credential || saved.credential_env ? { saved_credential: { key: saved.key, expected_revision: saved.revision } } : {}) }
    }
    if (!draft || pending.signal.aborted) return
    const result = await providerAdaptersApi.discoverCapabilities(draft.adapter, {
      selection: draft.config_json, ...(draft.api_key ? { api_key: draft.api_key } : {}),
      ...(draft.saved_credential ? { saved_credential: draft.saved_credential } : {}),
    }, pending.signal)
    if (!pending.signal.aborted) emit('discovered', result)
  } catch (cause) { if (!pending.signal.aborted) error.value = formatApiError(cause) }
  finally { if (controller === pending) busy.value = false }
}
</script>

<template>
  <div class="space-y-2">
    <Button variant="outline" :disabled="busy" @click="discover">{{ busy ? 'Đang tải giọng…' : 'Tải danh sách giọng' }}</Button>
    <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
  </div>
</template>

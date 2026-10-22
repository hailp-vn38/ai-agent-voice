<script setup lang="ts">
import { computed, ref } from 'vue'
import { requestJson, jsonRequest } from '@/api/client'
import { formatApiError } from '@/api/errors'
import type { AdminJob } from '@/api/types/agent-jobs'
import type { KnowledgeDraftState } from '@/api/types/knowledge-processing'
import { useI18n } from '@/composables/useI18n'
import { Button } from '@/components/ui/button'
const props = defineProps<{ job: AdminJob }>()
const emit = defineEmits<{ changed: [] }>()
const { t } = useI18n()
const error = ref('')
const pending = ref(false)
const confirmation = ref<{ state: KnowledgeDraftState; action: 'adopt' | 'discard' } | null>(null)
// Retain the request identity after a lost response; refresh never reuses it for a newer draft.
const requests = new Map<string, string>()
const source = computed(() => props.job.knowledge_state?.source_ref ?? (typeof props.job.result?.source_ref === 'string' ? props.job.result.source_ref : null))
const coverage = computed(() => props.job.result?.source_coverage as { read_bytes?: number; total_bytes?: number; complete?: boolean } | undefined)
const terminal = computed(() => ['succeeded', 'partial_success', 'failed', 'cancelled'].includes(props.job.status))
const knowledgeLink = (id: string, kind: 'source' | 'wiki') => ({ path: '/knowledge', query: { [kind]: id } })
async function resolve() {
  const selected = confirmation.value
  const draft = selected?.state.draft
  if (!selected || !draft || pending.value) return
  const identity = JSON.stringify([props.job.id, selected.state.wiki_id, draft.revision, draft.base_revision, draft.content_sha256, selected.action])
  const key = requests.get(identity) ?? crypto.randomUUID()
  requests.set(identity, key)
  pending.value = true
  error.value = ''
  try {
    await requestJson('/api/admin/knowledge/pages/' + encodeURIComponent(selected.state.wiki_id) + '/draft-resolution', jsonRequest('POST', {
      action: selected.action, owner_job_id: props.job.id,
      expected_draft_revision: draft.revision, expected_base_revision: draft.base_revision,
      expected_content_sha256: draft.content_sha256, operation_key: key,
    }))
    requests.delete(identity)
    confirmation.value = null
    emit('changed')
  } catch (cause) { error.value = formatApiError(cause); emit('changed') }
  finally { pending.value = false }
}
</script>
<template>
  <section class="studio-panel space-y-3 p-5">
    <h2 class="font-semibold">{{ t('jobUi.knowledge') }}</h2>
    <RouterLink v-if="source" :to="knowledgeLink(source, 'source')" class="text-sm underline">Source: {{ source }}</RouterLink>
    <p v-if="coverage" class="text-sm">{{ t('jobUi.sourceCoverage') }}: {{ coverage.read_bytes }} / {{ coverage.total_bytes }} bytes · {{ coverage.complete ? t('jobUi.llmCompleted') : t('jobUi.timelinePartial') }}</p>
    <p v-if="job.knowledge_state?.unavailable" role="alert" class="text-sm text-danger">{{ job.knowledge_state.code }}</p>
    <div v-for="state in job.knowledge_state?.drafts" :key="state.wiki_id" class="space-y-2 border-t pt-3 text-sm">
      <RouterLink :to="knowledgeLink(state.wiki_id, 'wiki')" class="underline">{{ state.wiki_id }}</RouterLink>
      <p v-if="state.draft">{{ t('jobUi.currentDraft') }}: {{ state.draft.revision }} · {{ state.draft.owner?.job_id ?? t('jobUi.adminOwned') }}</p>
      <p v-if="state.resolution">{{ state.resolution.action }} · {{ state.resolution.phase }}</p>
      <div v-if="terminal && state.draft?.owner?.job_id === job.id" class="flex gap-2">
        <Button variant="outline" :disabled="pending" @click="confirmation = { state, action: 'adopt' }">{{ t('jobUi.adoptDraft') }}</Button>
        <Button variant="outline" :disabled="pending" @click="confirmation = { state, action: 'discard' }">{{ t('jobUi.discardDraft') }}</Button>
      </div>
    </div>
    <div v-if="confirmation" class="space-y-3 rounded-lg border p-3">
      <p>{{ confirmation.action === 'adopt' ? t('jobUi.adoptConfirm') : t('jobUi.discardDraftConfirm') }}</p>
      <Button :disabled="pending" @click="resolve">{{ t('common.confirm') }}</Button>
      <Button variant="outline" :disabled="pending" @click="confirmation = null">{{ t('common.cancel') }}</Button>
    </div>
    <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
  </section>
</template>

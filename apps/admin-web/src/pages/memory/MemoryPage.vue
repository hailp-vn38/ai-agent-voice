<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { jsonRequest, requestJson, withQuery } from '@/api/client'
import { formatApiError } from '@/api/errors'
import PageHeader from '@/components/admin/PageHeader.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

type Hit = { id: string; title: string; revision: number; excerpt: string; status?: string; confirmed?: boolean }
type Fact = { id: string; title: string; body: string; revision: number; status: string; confirmed: boolean; source_ref: string | null }
type Health = { health: string; document_count: number; last_complete_scan_at: number | null }

const { t } = useI18n()
const q = ref('')
const hits = ref<Hit[]>([])
const health = ref<Health | null>(null)
const summary = ref('')
const selected = ref<Fact | null>(null)
const title = ref('')
const body = ref('')
const sourceRef = ref('')
const confirmed = ref(false)
const busy = ref(false)
const error = ref('')
const notice = ref('')

function newFact() {
  selected.value = null
  title.value = ''
  body.value = ''
  sourceRef.value = ''
  confirmed.value = false
  error.value = ''
  notice.value = ''
}
async function refresh() {
  try {
    const [h, s] = await Promise.all([
      requestJson<Health>('/api/admin/memory/index'),
      requestJson<{ summary: string }>('/api/admin/memory/summary'),
    ])
    health.value = h
    summary.value = s.summary
  } catch (cause) { error.value = formatApiError(cause) }
}
async function recall() {
  if (!q.value.trim()) { await loadAllFacts(); return }
  busy.value = true
  error.value = ''
  try {
    const result = await requestJson<{ items: Hit[] }>(
      withQuery('/api/admin/memory/facts', { q: q.value.trim(), limit: 20 }),
    )
    hits.value = result.items
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
async function loadAllFacts() {
  try {
    const response = await requestJson<{ items: Hit[] }>('/api/admin/memory/facts?limit=100')
    hits.value = response.items
  } catch (cause) { error.value = formatApiError(cause) }
}
async function openFact(id: string) {
  busy.value = true
  error.value = ''
  try {
    const fact = await requestJson<Fact>(`/api/admin/memory/facts/${encodeURIComponent(id)}`)
    selected.value = fact
    title.value = fact.title
    body.value = fact.body
    sourceRef.value = fact.source_ref ?? ''
    confirmed.value = fact.confirmed
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
async function save() {
  if (!title.value.trim() || !body.value.trim() || !sourceRef.value.trim() || busy.value) return
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    const isEdit = selected.value !== null
    const path = isEdit
      ? `/api/admin/memory/facts/${encodeURIComponent(selected.value!.id)}`
      : '/api/admin/memory/facts'
    const fact = await requestJson<Fact>(path, jsonRequest(isEdit ? 'PUT' : 'POST', {
      ...(isEdit ? { id: selected.value!.id } : {}),
      title: title.value,
      body: body.value,
      source_ref: sourceRef.value.trim(),
      confirmed: confirmed.value,
      expected_revision: selected.value?.revision ?? 0,
    }))
    selected.value = fact
    notice.value = t('memoryV3.saved')
    await refresh()
    if (q.value.trim()) await recall()
    else await loadAllFacts()
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
async function forget() {
  if (!selected.value || busy.value || !window.confirm(t('memoryV3.forgetConfirm'))) return
  busy.value = true
  error.value = ''
  try {
    await requestJson(`/api/admin/memory/facts/${encodeURIComponent(selected.value.id)}/forget`,
      jsonRequest('POST', { expected_revision: selected.value.revision }))
    newFact()
    notice.value = t('memoryV3.forgotten')
    await refresh()
    if (q.value.trim()) await recall()
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
async function rebuild() {
  busy.value = true
  error.value = ''
  try {
    await requestJson('/api/admin/memory/index/rebuild', { method: 'POST' })
    await refresh()
    notice.value = t('memoryV3.rebuilt')
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
onMounted(() => { void refresh(); void loadAllFacts() })
</script>

<template>
  <section class="space-y-5">
    <PageHeader :title="t('nav.memory')" :description="t('memoryV3.subtitle')">
      <template #actions>
        <Button variant="outline" :disabled="busy" @click="rebuild">{{ t('memoryV3.rebuild') }}</Button>
        <Button :disabled="busy" @click="newFact">{{ t('memoryV3.new') }}</Button>
      </template>
    </PageHeader>
    <div class="studio-panel flex flex-wrap items-center justify-between gap-3 p-4 text-sm">
      <span>{{ t('memoryV3.index') }}: <strong>{{ health?.health ?? '—' }}</strong></span>
      <span>{{ t('memoryV3.documents') }}: {{ health?.document_count ?? '—' }}</span>
      <span class="text-xs text-muted-foreground">{{ t('memoryV3.privateNotice') }}</span>
    </div>
    <p v-if="error" role="alert" class="rounded-lg border border-danger/30 p-3 text-sm text-danger">{{ error }}</p>
    <p v-if="notice" role="status" class="rounded-lg border border-success/30 p-3 text-sm text-success">{{ notice }}</p>
    <section class="studio-panel space-y-2 p-5">
      <h2 class="font-semibold">{{ t('memoryV3.summary') }}</h2>
      <p class="text-xs text-muted-foreground">{{ t('memoryV3.summaryNotice') }}</p>
      <pre class="max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted/40 p-3 text-sm">{{ summary }}</pre>
    </section>
    <div class="grid gap-4 lg:grid-cols-[minmax(250px,1fr)_minmax(0,2fr)]">
      <section class="studio-panel space-y-4 p-5">
        <h2 class="font-semibold">{{ t('memoryV3.facts') }}</h2>
        <form class="flex gap-2" @submit.prevent="recall">
          <input v-model="q" class="admin-input min-w-0 flex-1" :placeholder="t('memoryV3.recallHint')" />
          <Button type="submit" variant="outline" :disabled="busy">{{ t('memoryV3.recall') }}</Button>
        </form>
        <p v-if="!hits.length" class="text-sm text-muted-foreground">{{ t('memoryV3.empty') }}</p>
        <div v-for="hit in hits" :key="hit.id" class="space-y-1 border-b pb-3">
          <button type="button" class="text-left text-sm font-semibold hover:underline" :disabled="busy" @click="openFact(hit.id)">
            {{ hit.title }}
          </button>
          <p class="line-clamp-3 text-xs text-muted-foreground">{{ hit.excerpt }}</p>
          <span class="text-xs text-muted-foreground">r{{ hit.revision }} · {{ hit.status ?? 'active' }}</span>
        </div>
      </section>
      <form class="studio-panel space-y-4 p-5" @submit.prevent="save">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <h2 class="font-semibold">{{ selected ? t('memoryV3.edit') : t('memoryV3.new') }}</h2>
          <span v-if="selected" class="text-xs text-muted-foreground">{{ selected.id }} · r{{ selected.revision }}</span>
        </div>
        <label class="block space-y-1 text-sm">
          <span>{{ t('memoryV3.title') }}</span>
          <input v-model="title" class="admin-input w-full" maxlength="512" required />
        </label>
        <label class="block space-y-1 text-sm">
          <span>{{ t('memoryV3.source') }}</span>
          <input v-model="sourceRef" class="admin-input w-full" maxlength="1024" required placeholder="conversation:turn-id" />
        </label>
        <label class="block space-y-1 text-sm">
          <span>{{ t('memoryV3.markdown') }}</span>
          <textarea v-model="body" class="admin-input min-h-48 w-full font-mono text-sm" required />
        </label>
        <label class="flex items-center gap-2 text-sm">
          <input v-model="confirmed" type="checkbox" />
          {{ t('memoryV3.confirmed') }}
        </label>
        <div class="flex flex-wrap gap-2">
          <Button type="submit" :disabled="busy || selected?.status === 'forgotten' || !title.trim() || !body.trim() || !sourceRef.trim()">{{ t('memoryV3.save') }}</Button>
          <Button v-if="selected && selected.status !== 'forgotten'" type="button" variant="outline" class="border-danger/40 text-danger" :disabled="busy" @click="forget">
            {{ t('memoryV3.forget') }}
          </Button>
        </div>
      </form>
    </div>
  </section>
</template>

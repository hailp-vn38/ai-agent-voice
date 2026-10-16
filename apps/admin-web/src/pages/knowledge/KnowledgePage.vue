<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { requestJson, jsonRequest, withQuery } from '@/api/client'
import { formatApiError } from '@/api/errors'
import PageHeader from '@/components/admin/PageHeader.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

type Hit = { id: string; title: string; revision: number; excerpt: string; status?: string }
type Page = { id: string; title: string; body: string; revision: number; status: string; source_ref: string | null }
type Health = { health: string; document_count: number; last_complete_scan_at: number | null }
type Source = { id: string; version: number; name: string; provenance: string; sha256: string }

const { t } = useI18n()
const q = ref('')
const hits = ref<Hit[]>([])
const sources = ref<Source[]>([])
const sourceName = ref('')
const sourceProvenance = ref('')
const sourceContent = ref('')
const health = ref<Health | null>(null)
const selected = ref<Page | null>(null)
const title = ref('')
const body = ref('')
const sourceRef = ref('')
const busy = ref(false)
const error = ref('')
const notice = ref('')

function newDraft() {
  selected.value = null
  title.value = ''
  body.value = ''
  sourceRef.value = ''
  error.value = ''
  notice.value = ''
}
async function refreshSources() {
  try {
    const response = await requestJson<{ items: Source[] }>('/api/admin/knowledge/sources')
    sources.value = response.items
  } catch (cause) { error.value = formatApiError(cause) }
}
async function importSource() {
  if (!sourceName.value.trim() || !sourceContent.value.trim() || !sourceProvenance.value.trim() || busy.value) return
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    const source = await requestJson<Source>('/api/admin/knowledge/sources',
      jsonRequest('POST', { name: sourceName.value, provenance: sourceProvenance.value,
        content: sourceContent.value }))
    sourceRef.value = `${source.id}:${source.version}`
    sourceName.value = ''
    sourceContent.value = ''
    sourceProvenance.value = ''
    notice.value = t('knowledgeV3.sourceImported')
    await refreshSources()
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
async function publish() {
  if (!selected.value || selected.value.status !== 'draft' || !sourceRef.value.trim() || busy.value) return
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    const record = await requestJson<Page>(
      `/api/admin/knowledge/pages/${encodeURIComponent(selected.value.id)}/publish`,
      jsonRequest('POST', { expected_revision: selected.value.revision, source_ref: sourceRef.value.trim() }))
    selected.value = record
    notice.value = t('knowledgeV3.published')
    await refreshHealth()
    if (q.value.trim()) await search()
    else await listPages()
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
async function refreshHealth() {
  try { health.value = await requestJson<Health>('/api/admin/knowledge/index') }
  catch (cause) { error.value = formatApiError(cause) }
}
async function listPages() {
  try {
    const response = await requestJson<{ items: Hit[] }>('/api/admin/knowledge/pages?limit=100')
    hits.value = response.items
  } catch (cause) { error.value = formatApiError(cause) }
}
async function search() {
  if (!q.value.trim()) { await listPages(); return }
  busy.value = true
  error.value = ''
  try {
    const result = await requestJson<{ items: Hit[] }>(
      withQuery('/api/admin/knowledge/pages', { q: q.value.trim(), limit: 20 }),
    )
    hits.value = result.items
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
async function openPage(id: string) {
  busy.value = true
  error.value = ''
  try {
    const page = await requestJson<Page>(
      `/api/admin/knowledge/pages/${encodeURIComponent(id)}`,
    )
    selected.value = page
    title.value = page.title
    body.value = page.body
    sourceRef.value = page.source_ref ?? ''
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
async function save() {
  if (!title.value.trim() || !body.value.trim() || busy.value) return
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    const isEdit = selected.value !== null
    const path = isEdit
      ? `/api/admin/knowledge/pages/${encodeURIComponent(selected.value!.id)}`
      : '/api/admin/knowledge/pages'
    const page = await requestJson<Page>(path, jsonRequest(isEdit ? 'PUT' : 'POST', {
      ...(isEdit ? { id: selected.value!.id } : {}),
      title: title.value,
      body: body.value,
      source_ref: sourceRef.value.trim() || null,
      expected_revision: selected.value?.revision ?? 0,
    }))
    selected.value = page
    notice.value = t('knowledgeV3.saved')
    await refreshHealth()
    if (q.value.trim()) await search()
    else await listPages()
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
async function rebuild() {
  busy.value = true
  error.value = ''
  try {
    await requestJson('/api/admin/knowledge/index/rebuild', { method: 'POST' })
    await refreshHealth()
    notice.value = t('knowledgeV3.rebuilt')
  } catch (cause) { error.value = formatApiError(cause) }
  finally { busy.value = false }
}
onMounted(() => { void refreshHealth(); void refreshSources(); void listPages() })
</script>

<template>
  <section class="space-y-5">
    <PageHeader :title="t('nav.knowledge')" :description="t('knowledgeV3.subtitle')">
      <template #actions>
        <Button variant="outline" :disabled="busy" @click="rebuild">{{ t('knowledgeV3.rebuild') }}</Button>
        <Button :disabled="busy" @click="newDraft">{{ t('knowledgeV3.new') }}</Button>
      </template>
    </PageHeader>
    <div class="studio-panel flex flex-wrap items-center justify-between gap-3 p-4 text-sm">
      <span>{{ t('knowledgeV3.index') }}: <strong>{{ health?.health ?? '—' }}</strong></span>
      <span>{{ t('knowledgeV3.documents') }}: {{ health?.document_count ?? '—' }}</span>
      <span class="text-xs text-muted-foreground">{{ t('knowledgeV3.draftNotice') }}</span>
    </div>
    <p v-if="error" role="alert" class="rounded-lg border border-danger/30 p-3 text-sm text-danger">{{ error }}</p>
    <p v-if="notice" role="status" class="rounded-lg border border-success/30 p-3 text-sm text-success">{{ notice }}</p>

    <section class="studio-panel space-y-4 p-5">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <h2 class="font-semibold">{{ t('knowledgeV3.sources') }}</h2>
        <span class="text-xs text-muted-foreground">{{ t('knowledgeV3.markdownOnly') }}</span>
      </div>
      <form class="grid gap-3 md:grid-cols-[minmax(180px,1fr)_minmax(180px,1fr)]" @submit.prevent="importSource">
        <label class="space-y-1 text-sm"><span>{{ t('knowledgeV3.sourceName') }}</span>
          <input v-model="sourceName" class="admin-input w-full" maxlength="255" required placeholder="document.md" />
        </label>
        <label class="space-y-1 text-sm"><span>{{ t('knowledgeV3.sourceProvenance') }}</span>
          <input v-model="sourceProvenance" class="admin-input w-full" maxlength="1024" required placeholder="admin:manual" />
        </label>
        <label class="space-y-1 text-sm md:col-span-2"><span>{{ t('knowledgeV3.sourceContent') }}</span>
          <textarea v-model="sourceContent" class="admin-input min-h-28 w-full font-mono text-sm" required />
        </label>
        <div class="md:col-span-2"><Button :disabled="busy || !sourceName.trim() || !sourceContent.trim() || !sourceProvenance.trim()" type="submit">
          {{ t('knowledgeV3.importSource') }}
        </Button></div>
      </form>
      <div class="flex flex-wrap gap-2">
        <button v-for="source in sources" :key="source.id" type="button"
          class="rounded-lg border px-3 py-2 text-xs hover:bg-accent"
          @click="sourceRef = `${source.id}:${source.version}`">
          <strong>{{ source.name }}</strong> · {{ source.id }}:{{ source.version }}
        </button>
      </div>
    </section>
    <div class="grid gap-4 lg:grid-cols-[minmax(250px,1fr)_minmax(0,2fr)]">
      <section class="studio-panel space-y-4 p-5">
        <h2 class="font-semibold">{{ t('knowledgeV3.search') }}</h2>
        <form class="flex gap-2" @submit.prevent="search">
          <input v-model="q" class="admin-input min-w-0 flex-1" :placeholder="t('knowledgeV3.searchHint')" />
          <Button type="submit" variant="outline" :disabled="busy">{{ t('knowledgeV3.search') }}</Button>
        </form>
        <p v-if="!hits.length" class="text-sm text-muted-foreground">{{ t('knowledgeV3.empty') }}</p>
        <div v-for="item in hits" :key="item.id" class="space-y-1 border-b pb-3">
          <button type="button" class="text-left text-sm font-semibold hover:underline" :disabled="busy" @click="openPage(item.id)">
            {{ item.title }}
          </button>
          <p class="line-clamp-3 break-words text-xs text-muted-foreground">{{ item.excerpt }}</p>
          <span class="text-xs text-muted-foreground">r{{ item.revision }} · {{ item.status ?? "verified" }}</span>
        </div>
      </section>
      <form class="studio-panel space-y-4 p-5" @submit.prevent="save">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <h2 class="font-semibold">{{ selected ? t('knowledgeV3.edit') : t('knowledgeV3.new') }}</h2>
          <span v-if="selected" class="text-xs text-muted-foreground">{{ selected.id }} · r{{ selected.revision }}</span>
        </div>
        <label class="block space-y-1 text-sm">
          <span>{{ t('knowledgeV3.title') }}</span>
          <input v-model="title" class="admin-input w-full" maxlength="512" required />
        </label>
        <label class="block space-y-1 text-sm">
          <span>{{ t('knowledgeV3.source') }}</span>
          <input v-model="sourceRef" class="admin-input w-full" maxlength="1024" placeholder="source-id:revision" />
        </label>
        <label class="block space-y-1 text-sm">
          <span>{{ t('knowledgeV3.markdown') }}</span>
          <textarea v-model="body" class="admin-input min-h-80 w-full font-mono text-sm" required />
        </label>
        <div class="flex flex-wrap gap-2">
          <Button type="submit" :disabled="busy || !title.trim() || !body.trim() ">{{ t('knowledgeV3.save') }}</Button>
          <Button v-if="selected && selected.status === 'draft'" type="button" variant="outline"
            :disabled="busy || !sourceRef.trim()" @click="publish">{{ t('knowledgeV3.publish') }}</Button>
        </div>
      </form>
    </div>
  </section>
</template>

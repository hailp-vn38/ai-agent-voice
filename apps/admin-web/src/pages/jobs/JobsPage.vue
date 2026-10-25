<script setup lang="ts">
import { Activity, ArrowRight, ClipboardList, Plus, RefreshCw, Search, Settings2, Workflow } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { agentJobsApi } from '@/api/agent-jobs'
import { formatApiError, isApiError } from '@/api/errors'
import type { AdminJob, JobStatus } from '@/api/types/agent-jobs'
import PageHeader from '@/components/admin/PageHeader.vue'
import BaseModal from '@/components/admin/BaseModal.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const router = useRouter()
const { t } = useI18n()
const jobs = ref<AdminJob[]>([])
const search = ref('')
const filter = ref('all')
const page = ref(1)
const total = ref(0)
const hasMore = ref(false)
const loading = ref(true)
const busy = ref(false)
const error = ref('')
const notice = ref('')
const backendPending = ref(false)
const createOpen = ref(false)
const goal = ref('')
const statusOptions: JobStatus[] = ['queued', 'planning', 'running', 'cancelling', 'succeeded', 'partial_success', 'failed', 'cancelled']
let controller: AbortController | undefined
let creation = { goal: '', key: '' }

const activeCount = computed(() => jobs.value.filter(job => ['queued', 'planning', 'running', 'cancelling'].includes(job.status)).length)
const completedCount = computed(() => jobs.value.filter(job => ['succeeded', 'partial_success'].includes(job.status)).length)
const failedCount = computed(() => jobs.value.filter(job => job.status === 'failed').length)
const tone = (status: JobStatus): 'success' | 'danger' | 'secondary' | 'outline' =>
  status === 'succeeded' ? 'success' : status === 'failed' ? 'danger' :
    status === 'partial_success' || status === 'cancelled' ? 'outline' : 'secondary'
const label = (status: JobStatus) => t(('jobUi.jobStatus.' + status) as 'jobUi.jobStatus.queued')
function date(value: string) {
  const stamp = new Date(value)
  return Number.isFinite(stamp.getTime()) ? stamp.toLocaleString() : value
}
async function load(nextPage = 1) {
  controller?.abort()
  const pending = new AbortController()
  controller = pending
  loading.value = true
  error.value = ''
  try {
    const result = await agentJobsApi.listJobs({
      page: nextPage,
      status: filter.value === 'all' ? undefined : filter.value,
      q: search.value.trim() || undefined,
    }, pending.signal)
    if (pending.signal.aborted) return
    jobs.value = result.items
    total.value = result.total ?? result.items.length
    page.value = result.page
    hasMore.value = (result.page * result.page_size) < (result.total ?? (result.page * result.page_size + (result.items.length === result.page_size ? 1 : 0)))
    backendPending.value = false
  } catch (cause) {
    if (pending.signal.aborted) return
    backendPending.value = isApiError(cause) && cause.status === 404
    error.value = formatApiError(cause)
  } finally {
    if (controller === pending) loading.value = false
  }
}
async function create() {
  if (goal.value.trim().length < 5 || busy.value) return
  busy.value = true
  error.value = ''
  notice.value = ''
  try {
    if (creation.goal !== goal.value.trim()) creation = { goal: goal.value.trim(), key: crypto.randomUUID() }
    const job = await agentJobsApi.createJob(goal.value.trim(), creation.key)
    creation = { goal: '', key: '' }
    createOpen.value = false
    goal.value = ''
    await router.push({ name: 'job-detail', params: { jobId: job.id } })
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    busy.value = false
  }
}
function openDetail(id: string) {
  void router.push({ name: 'job-detail', params: { jobId: id } })
}
onMounted(() => { void load() })
onBeforeUnmount(() => { controller?.abort() })
</script>

<template>
  <section class="space-y-6">
    <PageHeader :title="t('jobUi.jobs')" :description="t('jobUi.jobsDescription')">
      <template #actions>
        <Button variant="outline" :disabled="loading" @click="load()"><RefreshCw class="size-4" :class="{ 'animate-spin': loading }" />{{ t('jobUi.refresh') }}</Button>
        <Button @click="createOpen = true"><Plus class="size-4" />{{ t('jobUi.newJob') }}</Button>
        <Button variant="outline" size="icon" :aria-label="t('jobUi.background')" :title="t('jobUi.background')" @click="router.push({ name: 'jobs-settings' })"><Settings2 class="size-4" aria-hidden="true" /></Button>
      </template>
    </PageHeader>
    <div v-if="error" role="alert" class="rounded-lg border border-danger/30 bg-danger/5 p-4 text-sm text-danger">
      <p class="font-medium">{{ backendPending ? t('jobUi.backendPending') : t('jobUi.loadFailed') }}</p><p class="mt-1 break-words">{{ error }}</p>
    </div>
    <p v-if="notice" role="status" class="rounded-lg border border-success/30 bg-success/5 p-3 text-sm text-success-foreground">{{ notice }}</p>

    <div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
      <div class="studio-panel flex items-center gap-3 p-4"><ClipboardList class="size-6 text-studio-violet" /><div><p class="text-xl font-semibold tabular-nums">{{ total }}</p><p class="text-xs text-muted-foreground">{{ t('jobUi.totalFiltered') }}</p></div></div>
      <div class="studio-panel flex items-center gap-3 p-4"><Activity class="size-6 text-studio-violet" /><div><p class="text-xl font-semibold tabular-nums">{{ activeCount }}</p><p class="text-xs text-muted-foreground">{{ t('jobUi.activeJobs') }}</p></div></div>
      <div class="studio-panel flex items-center gap-3 p-4"><Workflow class="size-6 text-success" /><div><p class="text-xl font-semibold tabular-nums">{{ completedCount }}</p><p class="text-xs text-muted-foreground">{{ t('jobUi.doneJobs') }}</p></div></div>
      <div class="studio-panel flex items-center gap-3 p-4"><ClipboardList class="size-6 text-danger" /><div><p class="text-xl font-semibold tabular-nums">{{ failedCount }}</p><p class="text-xs text-muted-foreground">{{ t('jobUi.failedJobs') }}</p></div></div>
    </div>

    <div class="flex flex-col gap-2 sm:flex-row">
      <form class="relative min-w-0 flex-1 sm:max-w-sm" @submit.prevent="load()">
        <Search class="pointer-events-none absolute left-3 top-3 size-4 text-muted-foreground" />
        <input v-model="search" class="admin-input pl-9" type="search" :placeholder="t('jobUi.searchJobs')" :aria-label="t('jobUi.searchJobs')" />
      </form>
      <select v-model="filter" class="admin-input sm:w-44" :aria-label="t('jobUi.filterStatus')" @change="load()">
        <option value="all">{{ t('jobUi.allStatus') }}</option>
        <option v-for="status in statusOptions" :key="status" :value="status">{{ label(status) }}</option>
      </select>
      <Button variant="outline" @click="load()">{{ t('jobUi.search') }}</Button>
    </div>

    <div v-if="loading" class="studio-panel p-8 text-center text-sm text-muted-foreground" role="status">{{ t('common.loading') }}</div>
    <div v-else-if="jobs.length" class="studio-panel overflow-hidden">
      <div class="overflow-x-auto">
        <table class="w-full text-left text-sm">
          <thead class="border-b bg-muted/40 text-xs text-muted-foreground">
            <tr>
              <th class="px-4 py-3 font-medium">{{ t('jobUi.goal') }}</th>
              <th class="px-4 py-3 font-medium">{{ t('jobUi.requester') }}</th>
              <th class="px-4 py-3 font-medium">{{ t('jobUi.status') }}</th>
              <th class="px-4 py-3 font-medium">{{ t('jobUi.createdAt') }}</th>
              <th class="px-4 py-3 font-medium"><span class="sr-only">{{ t('jobUi.details') }}</span></th>
            </tr>
          </thead>
          <tbody class="divide-y">
            <tr v-for="job in jobs" :key="job.id" class="hover:bg-muted/25">
              <td class="max-w-sm px-4 py-4">
                <button class="block w-full cursor-pointer truncate text-left font-medium hover:text-studio-violet" type="button" @click="openDetail(job.id)">{{ job.goal }}</button>
                <span v-if="job.execution_mode === 'knowledge_ingest_v3'" class="text-xs text-muted-foreground">{{ t('jobUi.knowledge') }}</span>
                <p class="mt-1 truncate font-mono text-xs text-muted-foreground">{{ job.id }}</p>
              </td>
              <td class="max-w-48 break-all px-4 py-4 text-muted-foreground">{{ job.requester_scope }}</td>
              <td class="px-4 py-4"><Badge :variant="tone(job.status)">{{ label(job.status) }}</Badge></td>
              <td class="whitespace-nowrap px-4 py-4 text-xs text-muted-foreground">{{ date(job.created_at) }}</td>
              <td class="px-4 py-4 text-right"><Button class="cursor-pointer" variant="ghost" size="icon" :aria-label="t('jobUi.details')" @click="openDetail(job.id)"><ArrowRight class="size-4" /></Button></td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
    <div v-else class="studio-panel p-10 text-center text-sm text-muted-foreground">
      <ClipboardList class="mx-auto mb-3 size-8 opacity-60" />{{ t('jobUi.noJobs') }}
    </div>
    <div class="flex justify-end gap-2">
      <Button variant="outline" :disabled="page <= 1 || loading" @click="load(page - 1)">{{ t('jobUi.previous') }}</Button>
      <Button variant="outline" :disabled="!hasMore || loading" @click="load(page + 1)">{{ t('jobUi.next') }}</Button>
    </div>

    <BaseModal v-model="createOpen" :title="t('jobUi.newJob')" :description="t('jobUi.createHint')">
      <form class="space-y-4" @submit.prevent="create">
        <label class="block space-y-2">
          <span class="text-sm font-medium">{{ t('jobUi.goal') }}</span>
          <textarea v-model="goal" class="admin-textarea min-h-32" maxlength="3000" required :placeholder="t('jobUi.goalPlaceholder')" />
        </label>
        <p class="text-xs text-muted-foreground">{{ t('jobUi.adminRequester') }}</p>
        <p v-if="error" role="alert" class="text-sm text-danger">{{ error }}</p>
        <div class="flex justify-end gap-2">
          <Button variant="outline" type="button" @click="createOpen = false">{{ t('jobUi.close') }}</Button>
          <Button type="submit" :disabled="busy || goal.trim().length < 5"><Plus class="size-4" />{{ t('jobUi.create') }}</Button>
        </div>
      </form>
    </BaseModal>
  </section>
</template>

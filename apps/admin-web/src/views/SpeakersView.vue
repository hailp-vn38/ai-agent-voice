<script setup lang="ts">
import { Plus, RefreshCw, Search } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { speakersApi } from '@/api/speakers'
import type { SpeakerSummary } from '@/api/types/speakers'
import BaseModal from '@/components/admin/BaseModal.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import PageHeader from '@/components/admin/PageHeader.vue'
import SpeakerCard from '@/components/speakers/SpeakerCard.vue'
import SpeakerEnrollmentWizard from '@/components/speakers/SpeakerEnrollmentWizard.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const router = useRouter()
const { t } = useI18n()

const items = ref<SpeakerSummary[]>([])
const total = ref(0)
const page = ref(1)
const pageSize = 50
const loading = ref(false)
const error = ref('')
const search = ref('')
let activeRequest: AbortController | undefined

const createOpen = ref(false)
const enrollmentOpen = ref(false)
const saving = ref(false)
const form = ref({ key: '', name: '', description: '' })
const deleteTarget = ref<SpeakerSummary>()
const deleteDialogOpen = computed({
  get: () => Boolean(deleteTarget.value),
  set: (open: boolean) => { if (!open) deleteTarget.value = undefined },
})

const filtered = computed(() => {
  const query = search.value.trim().toLocaleLowerCase()
  if (!query) return items.value
  return items.value.filter((speaker) =>
    [speaker.name, speaker.description ?? '', speaker.key]
      .some((value) => value.toLocaleLowerCase().includes(query)),
  )
})
const pageCount = computed(() => Math.max(1, Math.ceil(total.value / pageSize)))

async function load() {
  activeRequest?.abort()
  const controller = new AbortController()
  activeRequest = controller
  loading.value = true
  error.value = ''
  try {
    const result = await speakersApi.list({ page: page.value, pageSize, sort: '-updated_at' }, controller.signal)
    if (controller.signal.aborted) return
    items.value = result.items
    total.value = result.total
  } catch (cause) {
    if (!controller.signal.aborted) error.value = formatApiError(cause)
  } finally {
    if (activeRequest === controller) {
      loading.value = false
      activeRequest = undefined
    }
  }
}

function changePage(direction: -1 | 1) {
  const next = page.value + direction
  if (next < 1 || next > pageCount.value) return
  page.value = next
  void load()
}

function openCreate() {
  form.value = { key: '', name: '', description: '' }
  createOpen.value = true
}

function openEdit(key: string) {
  void router.push({ name: 'speaker-detail', params: { speakerKey: key }, query: { edit: '1' } })
}

async function enrollmentCompleted(created: { key: string }) {
  enrollmentOpen.value = false
  await router.push({ name: 'speaker-detail', params: { speakerKey: created.key } })
}

async function submitCreate() {
  if (saving.value) return
  saving.value = true
  error.value = ''
  try {
    const created = await speakersApi.create({
      key: form.value.key.trim(),
      name: form.value.name.trim(),
      description: form.value.description.trim() || undefined,
    })
    createOpen.value = false
    await router.push({ name: 'speaker-detail', params: { speakerKey: created.key } })
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

async function confirmDelete() {
  const target = deleteTarget.value
  if (!target || saving.value) return
  saving.value = true
  error.value = ''
  try {
    await speakersApi.remove(target.key, target.revision)
    deleteTarget.value = undefined
    if (items.value.length === 1 && page.value > 1) page.value -= 1
    await load()
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

onMounted(() => { void load() })
onBeforeUnmount(() => activeRequest?.abort())
</script>

<template>
  <section class="space-y-6">
    <PageHeader :eyebrow="t('speakers.eyebrow')" :title="t('speakers.title')" :description="t('speakers.description')">
      <template #actions>
        <Button variant="outline" :disabled="loading" @click="load">
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" />
          {{ t('common.refresh') }}
        </Button>
        <Button @click="enrollmentOpen = true">
          <Plus class="size-4" />
          {{ t('speakers.create') }}
        </Button>
        <Button variant="outline" @click="openCreate">{{ t('speakers.createTitle') }}</Button>
      </template>
    </PageHeader>

    <label class="relative block max-w-xl">
      <span class="sr-only">{{ t('speakers.search') }}</span>
      <Search class="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
      <input v-model="search" class="admin-input pl-9" type="search" :placeholder="t('speakers.search')" />
    </label>

    <div v-if="error" role="alert" class="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive">
      <span>{{ error }}</span>
      <Button size="sm" variant="outline" @click="load">{{ t('common.retry') }}</Button>
    </div>

    <div v-if="loading" class="grid gap-4 sm:grid-cols-2 xl:grid-cols-3" aria-busy="true">
      <div v-for="slot in 6" :key="slot" class="studio-panel h-52 animate-pulse bg-muted/20" />
    </div>

    <div v-else-if="!filtered.length" class="studio-panel px-6 py-12 text-center">
      <p class="text-sm text-muted-foreground">{{ search.trim() ? t('speakers.emptySearch') : t('speakers.empty') }}</p>
      <Button v-if="!search.trim() && !error" class="mt-4" @click="enrollmentOpen = true">
        <Plus class="size-4" />{{ t('speakers.create') }}
      </Button>
    </div>

    <div v-else class="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
      <SpeakerCard
        v-for="speaker in filtered"
        :key="speaker.key"
        :speaker="speaker"
        @edit="openEdit"
        @delete="deleteTarget = $event"
      />
    </div>

    <nav v-if="!loading && pageCount > 1" class="flex items-center justify-end gap-2" :aria-label="t('speakers.title')">
      <Button variant="outline" size="sm" :disabled="page <= 1" @click="changePage(-1)">{{ t('common.previous') }}</Button>
      <span class="text-sm text-muted-foreground">{{ page }} / {{ pageCount }}</span>
      <Button variant="outline" size="sm" :disabled="page >= pageCount" @click="changePage(1)">{{ t('common.next') }}</Button>
    </nav>

    <SpeakerEnrollmentWizard v-model:open="enrollmentOpen" @completed="enrollmentCompleted" />

    <BaseModal v-model="createOpen" :title="t('speakers.createTitle')">
      <form id="speaker-create-form" class="space-y-4" @submit.prevent="submitCreate">
        <label class="block space-y-1">
          <span class="text-sm font-medium">{{ t('speakers.key') }}</span>
          <input v-model="form.key" class="admin-input" required pattern="[A-Za-z0-9][A-Za-z0-9_-]*" />
        </label>
        <label class="block space-y-1">
          <span class="text-sm font-medium">{{ t('speakers.name') }}</span>
          <input v-model="form.name" class="admin-input" required maxlength="128" />
        </label>
        <label class="block space-y-1">
          <span class="text-sm font-medium">{{ t('speakers.descriptionField') }}</span>
          <textarea v-model="form.description" class="admin-textarea" rows="3" maxlength="2048" />
        </label>
      </form>
      <template #footer>
        <div class="flex justify-end gap-2">
          <Button variant="outline" @click="createOpen = false">{{ t('common.cancel') }}</Button>
          <Button type="submit" form="speaker-create-form" :disabled="saving">{{ t('speakers.createSubmit') }}</Button>
        </div>
      </template>
    </BaseModal>

    <ConfirmDialog
      v-model="deleteDialogOpen"
      :title="t('speakers.deleteTitle', { name: deleteTarget?.name ?? '' })"
      :description="t('speakers.deleteDescription')"
      :confirm-label="t('speakers.deleteSubmit')"
      tone="danger"
      @confirm="confirmDelete"
    />
  </section>
</template>

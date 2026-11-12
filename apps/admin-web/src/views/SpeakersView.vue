<script setup lang="ts">
import { AudioLines, BookOpen, ChevronLeft, ChevronRight, Mic, Plus, RefreshCw, Search, UserCheck, Users, X } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import CardListSkeleton from '@/components/admin/CardListSkeleton.vue'
import DirectoryHero from '@/components/admin/DirectoryHero.vue'
import { formatApiError } from '@/api/errors'
import { speakersApi } from '@/api/speakers'
import type { SpeakerSummary } from '@/api/types/speakers'
import BaseModal from '@/components/admin/BaseModal.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import SpeakerCard from '@/components/speakers/SpeakerCard.vue'
import SpeakerEnrollmentWizard from '@/components/speakers/SpeakerEnrollmentWizard.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useErrorToast } from '@/composables/useNotify'

const router = useRouter()
const { t } = useI18n()

const items = ref<SpeakerSummary[]>([])
const total = ref(0)
const page = ref(1)
const pageSize = 50
const loading = ref(false)
const error = ref('')
useErrorToast(() => error.value, { retry: () => load() })
const search = ref('')
const statusFilter = ref<'all' | 'enabled' | 'disabled'>('all')
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
  return items.value.filter((speaker) =>
    (statusFilter.value === 'all' || speaker.enabled === (statusFilter.value === 'enabled')) &&
    (!query || [speaker.name, speaker.description ?? '', speaker.key]
      .some((value) => value.toLocaleLowerCase().includes(query))),
  )
})
const enabledOnPage = computed(() => items.value.filter((item) => item.enabled).length)
const heroStats = computed(() => [
  { label: t('directoryUi.totalProfiles'), value: total.value, icon: Users },
  { label: t('directoryUi.enabledOnPage'), value: enabledOnPage.value, icon: UserCheck },
  { label: t('directoryUi.page'), value: `${page.value} / ${pageCount.value}`, icon: BookOpen },
])
const statusOptions = computed(() => [
  { value: 'all' as const, label: t('common.all'), count: items.value.length, tone: '' },
  { value: 'enabled' as const, label: t('speakers.enabled'), count: enabledOnPage.value, tone: 'on' },
  { value: 'disabled' as const, label: t('speakers.disabled'), count: items.value.length - enabledOnPage.value, tone: 'off' },
])
const hasFilters = computed(() => Boolean(search.value.trim()) || statusFilter.value !== 'all')
function clearFilters() { search.value = ''; statusFilter.value = 'all' }
const pageCount = computed(() => Math.max(1, Math.ceil(total.value / pageSize)))

async function load() {
  activeRequest?.abort()
  const controller = new AbortController()
  activeRequest = controller
  loading.value = true
  error.value = ''
  try {
    const result = await speakersApi.list({ page: page.value, pageSize, sort: 'updated_at' }, controller.signal)
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
  <section class="studio-directory" data-directory="speakers">
    <DirectoryHero tone="speakers" :eyebrow="t('directoryUi.speakersEyebrow')" :title="t('speakers.title')" :intro="t('directoryUi.speakersIntro')" :stats="heroStats">
      <template #actions>
        <button type="button" class="hero-primary" @click="enrollmentOpen = true"><Mic :size="16" aria-hidden="true" />{{ t('speakers.create') }}</button>
        <button type="button" class="hero-ghost" @click="openCreate"><Plus :size="16" aria-hidden="true" />{{ t('directoryUi.createEmptyProfile') }}</button>
        <button type="button" class="hero-ghost" :disabled="loading" :aria-label="t('common.refresh')" :title="t('common.refresh')" @click="load"><RefreshCw :size="16" :class="{ 'animate-spin': loading }" aria-hidden="true" /></button>
      </template>
    </DirectoryHero>

    <div class="directory-toolbar">
      <label class="directory-search"><span class="sr-only">{{ t('speakers.search') }}</span><Search :size="16" aria-hidden="true" /><input v-model="search" type="search" class="admin-input" :placeholder="pageCount > 1 ? t('directoryUi.pageSearchHint', { page }) : t('speakers.search')" /></label>
      <div class="directory-chips" role="group" :aria-label="t('speakers.title')">
        <button v-for="option in statusOptions" :key="option.value" type="button" class="directory-chip" :aria-pressed="statusFilter === option.value" @click="statusFilter = option.value">
          <i v-if="option.tone" :data-tone="option.tone" aria-hidden="true" />{{ option.label }}<b>{{ option.count }}</b>
        </button>
      </div>
      <button v-if="hasFilters" type="button" class="directory-clear" @click="clearFilters"><X :size="14" aria-hidden="true" />{{ t('providers.clearFilters') }}</button>
      <span v-if="!loading" class="directory-count">{{ t('providers.resultCount', { count: filtered.length }) }}</span>
    </div>

    <CardListSkeleton v-if="loading" class="sm:grid-cols-2 xl:grid-cols-3" />

    <div v-else-if="!filtered.length" class="directory-empty text-center">
      <span class="directory-empty-icon"><AudioLines :size="28" :stroke-width="1.6" aria-hidden="true" /></span>
      <p>{{ hasFilters ? t('speakers.emptySearch') : t('speakers.empty') }}</p>
      <Button v-if="hasFilters" variant="outline" @click="clearFilters"><X class="size-4" aria-hidden="true" />{{ t('providers.clearFilters') }}</Button>
      <Button v-else-if="!error" @click="enrollmentOpen = true">
        <Plus class="size-4" />{{ t('speakers.create') }}
      </Button>
    </div>

    <div v-else class="directory-grid">
      <SpeakerCard
        v-for="speaker in filtered"
        :key="speaker.key"
        :speaker="speaker"
        @edit="openEdit"
        @delete="deleteTarget = $event"
      />
    </div>

    <nav v-if="!loading && pageCount > 1" class="directory-pagination" :aria-label="t('speakers.title')"><span>{{ t('directoryUi.page') }} {{ page }} / {{ pageCount }}</span><div><Button variant="outline" size="sm" :disabled="page <= 1" @click="changePage(-1)"><ChevronLeft class="size-4" aria-hidden="true" />{{ t('common.previous') }}</Button><Button variant="outline" size="sm" :disabled="page >= pageCount" @click="changePage(1)">{{ t('common.next') }}<ChevronRight class="size-4" aria-hidden="true" /></Button></div></nav>

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

<style scoped src="./studio-directory.css"></style>

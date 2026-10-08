<script setup lang="ts">
import { Mic, Plus, RefreshCw, Search } from '@lucide/vue'
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { speakersApi } from '@/api/speakers'
import type { SpeakerSummary } from '@/api/types/speakers'
import SpeakerEnrollmentWizard from '@/components/speakers/SpeakerEnrollmentWizard.vue'
import BaseModal from '@/components/admin/BaseModal.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import PageHeader from '@/components/admin/PageHeader.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const router = useRouter()
const { t, formatDateTime } = useI18n()

const items = ref<SpeakerSummary[]>([])
const total = ref(0)
const page = ref(1)
const pageSize = 50
const loading = ref(false)
const error = ref('')
const search = ref('')

const createOpen = ref(false)
const enrollmentOpen = ref(false)
const saving = ref(false)
const form = ref({ key: '', name: '', description: '' })
const deleteTarget = ref<SpeakerSummary | undefined>()
const deleteDialogOpen = computed({
  get: () => Boolean(deleteTarget.value),
  set: (value: boolean) => {
    if (!value) deleteTarget.value = undefined
  },
})

const filtered = computed(() => {
  const query = search.value.trim().toLowerCase()
  if (!query) return items.value
  return items.value.filter(
    (speaker) =>
      speaker.key.toLowerCase().includes(query) || speaker.name.toLowerCase().includes(query),
  )
})

const pageCount = computed(() => Math.max(1, Math.ceil(total.value / pageSize)))

async function load() {
  loading.value = true
  error.value = ''
  try {
    const result = await speakersApi.list({ page: page.value, pageSize })
    items.value = result.items
    total.value = result.total
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    loading.value = false
  }
}

function openCreate() {
  form.value = { key: '', name: '', description: '' }
  createOpen.value = true
}

async function enrollmentCompleted(created: { key: string }) {
  enrollmentOpen.value = false
  await load()
  await router.push({ name: 'speaker-detail', params: { speakerKey: created.key } })
}

async function submitCreate() {
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
  if (!target) return
  saving.value = true
  error.value = ''
  try {
    await speakersApi.remove(target.key, target.revision)
    deleteTarget.value = undefined
    await load()
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

onMounted(load)
</script>

<template>
  <section class="space-y-6">
    <PageHeader :eyebrow="t('speakers.eyebrow')" :title="t('speakers.title')" :description="t('speakers.description')">
      <template #actions>
        <Button variant="outline" @click="load">
          <RefreshCw class="size-4" />
          {{ t('common.refresh') }}
        </Button>
        <Button @click="enrollmentOpen = true">
          <Plus class="size-4" />
          {{ t('speakers.create') }}
        </Button>
        <Button variant="outline" @click="openCreate">Tạo hồ sơ trống</Button>
      </template>
    </PageHeader>

    <div class="flex flex-wrap items-center gap-3">
      <label class="relative min-w-64 flex-1">
        <Search class="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
        <input v-model="search" class="admin-input pl-9" :placeholder="t('speakers.search')" />
      </label>
    </div>

    <p v-if="error" class="rounded-md border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive">
      {{ error }}
    </p>

    <div v-if="loading" class="py-10 text-center text-sm text-muted-foreground">
      {{ t('common.loading') }}
    </div>

    <div v-else-if="filtered.length === 0" class="rounded-lg border border-dashed px-6 py-12 text-center text-sm text-muted-foreground">
      <p>{{ t('speakers.empty') }}</p>
      <Button class="mt-4" @click="enrollmentOpen = true"><Plus class="size-4" />{{ t('speakers.create') }}</Button>
    </div>

    <div v-else class="overflow-hidden rounded-lg border">
      <table class="w-full text-sm">
        <thead class="bg-muted/40 text-left text-xs uppercase tracking-wide text-muted-foreground">
          <tr>
            <th class="px-4 py-3">{{ t('speakers.key') }}</th>
            <th class="px-4 py-3">{{ t('speakers.name') }}</th>
            <th class="px-4 py-3">{{ t('speakers.revision') }}</th>
            <th class="px-4 py-3">{{ t('speakers.updatedAt') }}</th>
            <th class="px-4 py-3 text-right">{{ t('common.actions') }}</th>
          </tr>
        </thead>
        <tbody class="divide-y">
          <tr v-for="speaker in filtered" :key="speaker.key" class="hover:bg-muted/30">
            <td class="px-4 py-3 font-mono text-xs">{{ speaker.key }}</td>
            <td class="px-4 py-3">
              <button class="flex items-center gap-2 text-left font-medium hover:underline" @click="router.push({ name: 'speaker-detail', params: { speakerKey: speaker.key } })">
                <Mic class="size-4 text-muted-foreground" />
                {{ speaker.name }}
              </button>
              <p v-if="speaker.description" class="mt-1 text-xs text-muted-foreground">{{ speaker.description }}</p>
            </td>
            <td class="px-4 py-3">{{ speaker.revision }}</td>
            <td class="px-4 py-3 text-muted-foreground">{{ formatDateTime(new Date(speaker.updated_at * 1000)) }}</td>
            <td class="px-4 py-3 text-right">
              <Button variant="ghost" size="sm" @click="router.push({ name: 'speaker-detail', params: { speakerKey: speaker.key } })">
                {{ t('common.open') }}
              </Button>
              <Button variant="ghost" size="sm" @click="deleteTarget = speaker">
                {{ t('common.delete') }}
              </Button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <div v-if="pageCount > 1" class="flex items-center justify-end gap-2">
      <Button variant="outline" size="sm" :disabled="page <= 1" @click="page--; load()">
        {{ t('common.previous') }}
      </Button>
      <span class="text-sm text-muted-foreground">{{ page }} / {{ pageCount }}</span>
      <Button variant="outline" size="sm" :disabled="page >= pageCount" @click="page++; load()">
        {{ t('common.next') }}
      </Button>
    </div>

    <SpeakerEnrollmentWizard v-model:open="enrollmentOpen" @completed="enrollmentCompleted" />

    <BaseModal v-model="createOpen" :title="t('speakers.createTitle')">
      <form class="space-y-4" @submit.prevent="submitCreate">
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
          <textarea v-model="form.description" class="admin-input" rows="3" maxlength="2048" />
        </label>
      </form>
      <template #footer>
        <div class="flex justify-end gap-2">
          <Button variant="outline" @click="createOpen = false">{{ t('common.cancel') }}</Button>
          <Button :disabled="saving" @click="submitCreate">{{ t('speakers.createSubmit') }}</Button>
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

<script setup lang="ts">
import { ArrowLeft, AudioLines, CheckCircle2, ChevronRight, Copy, Eraser, Mic, Pencil, RefreshCw, Trash2 } from '@lucide/vue'
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { speakersApi } from '@/api/speakers'
import type { Speaker, SpeakerVoiceprint } from '@/api/types/speakers'
import BaseModal from '@/components/admin/BaseModal.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import SpeakerEnrollmentWizard from '@/components/speakers/SpeakerEnrollmentWizard.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const route = useRoute()
const router = useRouter()
const { t, formatDateTime } = useI18n()

const speakerKey = computed(() => typeof route.params.speakerKey === 'string' ? route.params.speakerKey : '')
const speaker = ref<Speaker>()
const currentSpace = ref<string | null>(null)
const engineAvailable = ref(false)
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const copied = ref(false)
const editOpen = ref(false)
const deleteOpen = ref(false)
const purgeOpen = ref(false)
const enrollOpen = ref(false)
const editForm = ref({ name: '', description: '', enabled: true })
let activeRequest: AbortController | undefined

const voiceprintCount = computed(() => speaker.value?.voiceprints.length ?? 0)
const speakerStatusKey = computed(() => {
  if (!speaker.value?.enabled) return 'speakers.disabled' as const
  if (voiceprintCount.value > 0) return 'speakers.voiceSaved' as const
  if (speaker.value.enrollment_drafts.length) return 'speakers.statusDraft' as const
  return 'speakers.statusUnenrolled' as const
})

function openEdit() {
  if (!speaker.value) return
  editForm.value = {
    name: speaker.value.name,
    description: speaker.value.description ?? '',
    enabled: speaker.value.enabled,
  }
  editOpen.value = true
}

async function load() {
  activeRequest?.abort()
  const controller = new AbortController()
  activeRequest = controller
  speaker.value = undefined
  copied.value = false
  editOpen.value = false
  enrollOpen.value = false
  loading.value = true
  error.value = ''
  try {
    if (!speakerKey.value) throw new Error('Invalid speaker key')
    const [result, summary] = await Promise.all([
      speakersApi.get(speakerKey.value, controller.signal),
      speakersApi.summary(controller.signal),
    ])
    if (controller.signal.aborted) return
    speaker.value = result
    currentSpace.value = summary.embedding_space_id
    engineAvailable.value = summary.available
    if (route.query.edit === '1') {
      openEdit()
      void router.replace({
        name: 'speaker-detail',
        params: { speakerKey: result.key },
        query: { ...route.query, edit: undefined },
      })
    }
  } catch (cause) {
    if (!controller.signal.aborted) error.value = formatApiError(cause)
  } finally {
    if (activeRequest === controller) {
      loading.value = false
      activeRequest = undefined
    }
  }
}

async function saveEdit() {
  if (!speaker.value || saving.value || !editForm.value.name.trim()) return
  saving.value = true
  error.value = ''
  try {
    await speakersApi.update(speaker.value.key, {
      name: editForm.value.name.trim(),
      description: editForm.value.description.trim() || null,
      enabled: editForm.value.enabled,
    }, speaker.value.revision)
    // PATCH returns empty voiceprint/draft arrays; GET restores the full detail projection.
    editOpen.value = false
    await load()
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

async function deleteSpeaker() {
  if (!speaker.value || saving.value) return
  saving.value = true
  error.value = ''
  try {
    await speakersApi.remove(speaker.value.key, speaker.value.revision)
    await router.push({ name: 'speakers' })
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

async function purgeVoiceprint() {
  if (!speaker.value || saving.value) return
  saving.value = true
  error.value = ''
  try {
    speaker.value = await speakersApi.purgeVoiceprint(speaker.value.key, speaker.value.revision)
    purgeOpen.value = false
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

function enrollmentCompleted(updated: Speaker) {
  enrollOpen.value = false
  speaker.value = updated
  void load()
}

function validationLabel(voiceprint: SpeakerVoiceprint) {
  if (!engineAvailable.value) return 'Không xác định khả năng tương thích'
  return voiceprint.embedding_space_id === currentSpace.value ? 'Đã đăng ký' : 'Cần thu lại mẫu'
}

async function copyKey() {
  if (!speaker.value) return
  try {
    await navigator.clipboard.writeText(speaker.value.key)
    copied.value = true
  } catch {
    error.value = t('speakers.copyFailed')
  }
}

watch(speakerKey, () => { void load() }, { immediate: true })
onBeforeUnmount(() => activeRequest?.abort())
</script>

<template>
  <section class="space-y-6">
    <RouterLink :to="{ name: 'speakers' }" class="inline-flex items-center gap-2 text-sm text-muted-foreground transition hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring">
      <ArrowLeft class="size-4" aria-hidden="true" />
      {{ t('nav.speakers') }}
    </RouterLink>

    <div v-if="error" role="alert" class="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive">
      <span>{{ error }}</span>
      <Button v-if="!speaker" variant="outline" size="sm" @click="load">{{ t('common.retry') }}</Button>
    </div>

    <div v-if="loading" class="space-y-4" aria-busy="true">
      <div class="studio-panel h-36 animate-pulse bg-muted/20" />
      <div class="studio-panel h-64 animate-pulse bg-muted/20" />
    </div>

    <div v-else-if="!speaker" class="studio-panel px-5 py-12 text-center text-muted-foreground">
      <p class="text-sm">{{ t('speakers.detailUnavailable') }}</p>
      <RouterLink :to="{ name: 'speakers' }" class="mt-3 inline-block text-sm font-medium text-foreground underline underline-offset-4">{{ t('nav.speakers') }}</RouterLink>
    </div>

    <template v-else>
      <header class="studio-panel flex flex-col gap-5 p-5 sm:flex-row sm:items-start sm:justify-between sm:p-6">
        <div class="flex min-w-0 items-start gap-4">
          <span class="flex size-14 shrink-0 items-center justify-center rounded-2xl bg-studio-violet/10 text-studio-violet">
            <AudioLines class="size-7" aria-hidden="true" />
          </span>
          <div class="min-w-0">
            <p class="text-xs font-medium uppercase tracking-wider text-studio-violet">{{ t('speakers.eyebrow') }}</p>
            <h1 class="mt-1 break-words text-2xl font-semibold tracking-tight sm:text-3xl">{{ speaker.name }}</h1>
            <p v-if="speaker.description" class="mt-2 max-w-2xl whitespace-pre-wrap break-words text-sm text-muted-foreground">{{ speaker.description }}</p>
            <span
              class="mt-3 inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-medium"
              :class="speaker.enabled && voiceprintCount ? 'bg-success/10 text-success-foreground' : 'bg-muted text-muted-foreground'"
            >
              <CheckCircle2 v-if="speaker.enabled && voiceprintCount" class="size-3.5" aria-hidden="true" />
              {{ t(speakerStatusKey) }}
            </span>
          </div>
        </div>
        <div class="flex shrink-0 flex-wrap gap-2">
          <Button variant="outline" :disabled="loading" @click="load"><RefreshCw class="size-4" />{{ t('common.refresh') }}</Button>
          <Button variant="outline" @click="openEdit"><Pencil class="size-4" />{{ t('common.edit') }}</Button>
          <Button @click="enrollOpen = true"><Mic class="size-4" />{{ t('speakers.confirmVoice') }}</Button>
        </div>
      </header>

      <div class="grid gap-4 lg:grid-cols-[minmax(0,1.35fr)_minmax(0,1fr)]">
        <section class="studio-panel min-w-0 p-5 sm:p-6">
          <h2 class="text-base font-semibold">{{ t('speakers.profileDetails') }}</h2>
          <dl class="mt-5 grid gap-5 sm:grid-cols-2">
            <div class="min-w-0">
              <dt class="text-xs text-muted-foreground">{{ t('speakers.name') }}</dt>
              <dd class="mt-1 break-words text-sm font-medium">{{ speaker.name }}</dd>
            </div>
            <div class="min-w-0">
              <dt class="text-xs text-muted-foreground">{{ t('speakers.enabled') }}</dt>
              <dd class="mt-1 text-sm font-medium">{{ speaker.enabled ? t('speakers.enabled') : t('speakers.disabled') }}</dd>
            </div>
            <div class="min-w-0">
              <dt class="text-xs text-muted-foreground">{{ t('speakers.createdAt') }}</dt>
              <dd class="mt-1 text-sm font-medium">{{ formatDateTime(new Date(speaker.created_at * 1000)) }}</dd>
            </div>
            <div class="min-w-0">
              <dt class="text-xs text-muted-foreground">{{ t('speakers.updatedAt') }}</dt>
              <dd class="mt-1 text-sm font-medium">{{ formatDateTime(new Date(speaker.updated_at * 1000)) }}</dd>
            </div>
            <div class="min-w-0 sm:col-span-2">
              <dt class="text-xs text-muted-foreground">{{ t('speakers.descriptionField') }}</dt>
              <dd class="mt-1 whitespace-pre-wrap break-words text-sm">{{ speaker.description || t('speakers.noDescription') }}</dd>
            </div>
          </dl>
        </section>

        <section class="studio-panel min-w-0 p-5 sm:p-6">
          <h2 class="text-base font-semibold">{{ t('speakers.voiceStatus') }}</h2>
          <div class="mt-5 flex items-center gap-3">
            <span class="flex size-10 shrink-0 items-center justify-center rounded-lg bg-studio-cyan/10 text-studio-cyan"><AudioLines class="size-5" aria-hidden="true" /></span>
            <div class="min-w-0">
              <p class="text-sm font-medium">{{ t(speakerStatusKey) }}</p>
              <p class="mt-0.5 text-xs text-muted-foreground">{{ voiceprintCount ? t('speakers.voiceReadyHint') : t('speakers.voiceMissingHint') }}</p>
            </div>
          </div>
          <ul v-if="speaker.voiceprints.length" class="mt-4 space-y-2 border-t border-border/70 pt-4">
            <li v-for="voiceprint in speaker.voiceprints" :key="voiceprint.embedding_space_id" class="flex items-start gap-2 text-xs text-muted-foreground">
              <CheckCircle2 class="mt-0.5 size-3.5 shrink-0 text-success" aria-hidden="true" />
              <span>{{ validationLabel(voiceprint) }}</span>
            </li>
          </ul>
          <p class="mt-4 text-xs leading-relaxed text-muted-foreground">{{ t('speakers.identityNotice') }}</p>
        </section>
      </div>

      <section v-if="speaker.enrollment_drafts.length" class="studio-panel flex flex-wrap items-center justify-between gap-3 p-5">
        <div>
          <h2 class="text-sm font-semibold">{{ t('speakers.drafts') }}</h2>
          <p class="mt-1 text-sm text-muted-foreground">{{ t('speakers.draftNotice') }}</p>
        </div>
        <Button variant="outline" @click="enrollOpen = true">
          {{ t('speakers.resumeDraft') }}
          <ChevronRight class="size-4" aria-hidden="true" />
        </Button>
      </section>

      <details class="studio-panel group min-w-0 p-5">
        <summary class="flex cursor-pointer list-none items-center justify-between gap-2 text-sm font-semibold">
          {{ t('speakers.technicalDetails') }}
          <ChevronRight class="size-4 shrink-0 text-muted-foreground transition group-open:rotate-90" aria-hidden="true" />
        </summary>
        <div class="mt-4 flex flex-wrap items-end justify-between gap-3 border-t border-border/70 pt-4">
          <div class="min-w-0 flex-1">
            <p class="text-xs text-muted-foreground">{{ t('speakers.key') }}</p>
            <p class="mt-1 break-all font-mono text-xs">{{ speaker.key }}</p>
          </div>
          <Button variant="outline" size="sm" @click="copyKey">
            <Copy class="size-3.5" aria-hidden="true" />
            {{ copied ? t('speakers.copiedKey') : t('speakers.copyKey') }}
          </Button>
        </div>
        <p class="mt-3 text-xs text-muted-foreground">{{ t('speakers.revision') }}: {{ speaker.revision }}</p>
      </details>

      <section class="studio-panel flex flex-col gap-4 p-5 sm:flex-row sm:items-center sm:justify-between">
        <div class="min-w-0">
          <h2 class="text-sm font-semibold">{{ t('speakers.dangerZone') }}</h2>
          <p class="mt-1 text-xs text-muted-foreground">{{ t('speakers.dangerHint') }}</p>
        </div>
        <div class="flex flex-wrap gap-2">
          <Button v-if="speaker.voiceprints.length" variant="outline" :disabled="saving" @click="purgeOpen = true">
            <Eraser class="size-4" />{{ t('speakers.purge') }}
          </Button>
          <Button variant="outline" class="text-danger-foreground hover:text-danger-foreground" :disabled="saving" @click="deleteOpen = true">
            <Trash2 class="size-4" />{{ t('common.delete') }}
          </Button>
        </div>
      </section>
    </template>

    <SpeakerEnrollmentWizard v-model:open="enrollOpen" :speaker-key="speakerKey" @completed="enrollmentCompleted" />
    <BaseModal v-model="editOpen" :title="t('speakers.editTitle')">
      <form id="speaker-edit-form" class="space-y-4" @submit.prevent="saveEdit">
        <label class="block space-y-1">
          <span class="text-sm font-medium">{{ t('speakers.name') }}</span>
          <input v-model="editForm.name" class="admin-input" required maxlength="128" />
        </label>
        <label class="block space-y-1">
          <span class="text-sm font-medium">{{ t('speakers.descriptionField') }}</span>
          <textarea v-model="editForm.description" class="admin-textarea" rows="3" maxlength="2048" />
        </label>
        <label class="flex items-center gap-2 text-sm">
          <input v-model="editForm.enabled" type="checkbox" />{{ t('speakers.enabled') }}
        </label>
      </form>
      <template #footer>
        <div class="flex justify-end gap-2">
          <Button variant="outline" @click="editOpen = false">{{ t('common.cancel') }}</Button>
          <Button type="submit" form="speaker-edit-form" :disabled="saving || !editForm.name.trim()">{{ t('common.saveChanges') }}</Button>
        </div>
      </template>
    </BaseModal>
    <ConfirmDialog v-model="deleteOpen" :title="t('speakers.deleteTitle', { name: speaker?.name ?? '' })" :description="t('speakers.deleteDescription')" :confirm-label="t('speakers.deleteSubmit')" tone="danger" @confirm="deleteSpeaker" />
    <ConfirmDialog v-model="purgeOpen" :title="t('speakers.purgeTitle', { name: speaker?.name ?? '' })" :description="t('speakers.purgeDescription')" :confirm-label="t('speakers.purgeSubmit')" tone="danger" @confirm="purgeVoiceprint" />
  </section>
</template>

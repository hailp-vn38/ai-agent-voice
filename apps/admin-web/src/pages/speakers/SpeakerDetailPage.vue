<script setup lang="ts">
import { ArrowLeft, Mic, Play, RefreshCw, Square, Trash2, X } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { formatApiError } from '@/api/errors'
import { providersApi } from '@/api/providers'
import { speakersApi } from '@/api/speakers'
import type { AdminProvider } from '@/api/types/providers'
import type { EnrollmentDraft, EnrollmentValidationStatus, Speaker } from '@/api/types/speakers'
import BaseModal from '@/components/admin/BaseModal.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import PageHeader from '@/components/admin/PageHeader.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useMicrophoneRecorder } from '@/composables/useMicrophoneRecorder'

const route = useRoute()
const router = useRouter()
const { t, formatDateTime } = useI18n()

const speakerKey = computed(() => String(route.params.speakerKey))
const speaker = ref<Speaker>()
const loading = ref(false)
const saving = ref(false)
const error = ref('')

const editOpen = ref(false)
const editForm = ref({ name: '', description: '', enabled: true })
const deleteOpen = ref(false)

const providers = ref<AdminProvider[]>([])
const enrollOpen = ref(false)
const selectedProviderKey = ref('')
const draft = ref<EnrollmentDraft>()
const uploading = ref(false)
const sampleError = ref('')
const previewUrl = ref('')
let uploadAbort: AbortController | undefined
const enrollmentLimits = ref({ minClipMs: 5_000, maxClipMs: 10_000 })
const maxSamples = ref(5)
const minSamples = ref(3)
const recorder = useMicrophoneRecorder()
const holdoutRecorder = useMicrophoneRecorder()
const validating = ref(false)
const finalizing = ref(false)
const validationError = ref('')

const nextSlot = computed(() => {
  const used = draft.value?.samples.map((sample) => sample.slot) ?? []
  return used.length === 0 ? 1 : Math.max(...used) + 1
})
const canRecord = computed(
  () => Boolean(draft.value) && nextSlot.value <= maxSamples.value && !uploading.value,
)
const canValidate = computed(
  () =>
    Boolean(draft.value) &&
    (draft.value?.samples.length ?? 0) >= minSamples.value &&
    !uploading.value &&
    !validating.value &&
    !holdoutRecorder.recording.value,
)
const canFinalize = computed(
  () => Boolean(draft.value?.validation.valid_for_current_revision) && !finalizing.value,
)
const validationLabel = computed(() => {
  switch (draft.value?.validation.status) {
    case 'passed':
      return t('speakers.validationPassed')
    case 'failed':
      return t('speakers.validationFailed')
    case 'inconsistent':
      return t('speakers.validationInconsistent')
    case 'ambiguous':
      return t('speakers.validationAmbiguous')
    default:
      return t('speakers.validationNone')
  }
})

const selectedProvider = computed(() =>
  providers.value.find((provider) => provider.key === selectedProviderKey.value),
)

async function load() {
  loading.value = true
  error.value = ''
  try {
    speaker.value = await speakersApi.get(speakerKey.value)
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    loading.value = false
  }
}

function openEdit() {
  if (!speaker.value) return
  editForm.value = {
    name: speaker.value.name,
    description: speaker.value.description ?? '',
    enabled: speaker.value.enabled,
  }
  editOpen.value = true
}

async function submitEdit() {
  if (!speaker.value) return
  saving.value = true
  error.value = ''
  try {
    speaker.value = await speakersApi.update(
      speaker.value.key,
      {
        name: editForm.value.name.trim(),
        description: editForm.value.description.trim() || null,
        enabled: editForm.value.enabled,
      },
      speaker.value.revision,
    )
    editOpen.value = false
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

async function confirmDelete() {
  if (!speaker.value) return
  saving.value = true
  error.value = ''
  try {
    await speakersApi.remove(speaker.value.key, speaker.value.revision)
    await router.push({ name: 'speakers' })
  } catch (cause) {
    error.value = formatApiError(cause)
    deleteOpen.value = false
  } finally {
    saving.value = false
  }
}

async function openEnroll() {
  error.value = ''
  draft.value = undefined
  selectedProviderKey.value = ''
  enrollOpen.value = true
  try {
    const [page, summary] = await Promise.all([
      providersApi.list({ type: 'speaker', pageSize: 100 }),
      speakersApi.summary(),
    ])
    providers.value = page.items.filter((provider) => provider.enabled === 1)
    const enrollment = summary.enrollment
    if (enrollment) {
      enrollmentLimits.value = {
        minClipMs: enrollment.min_clip_ms,
        maxClipMs: enrollment.max_clip_ms,
      }
      minSamples.value = enrollment.min_samples
      maxSamples.value = enrollment.max_samples
    }
  } catch (cause) {
    error.value = formatApiError(cause)
  }
}

async function startDraft() {
  if (!speaker.value || !selectedProvider.value) return
  saving.value = true
  error.value = ''
  try {
    draft.value = await speakersApi.createDraft(
      speaker.value.key,
      {
        provider_key: selectedProvider.value.key,
        expected_provider_revision: selectedProvider.value.revision,
      },
      speaker.value.revision,
    )
  } catch (cause) {
    // A lost response can leave an open draft behind; surface the error and let the user resume
    // from the drafts list rather than opening a second draft.
    error.value = formatApiError(cause)
    void load()
  } finally {
    saving.value = false
  }
}

async function resumeDraft(id: string) {
  saving.value = true
  error.value = ''
  try {
    draft.value = await speakersApi.getDraft(speakerKey.value, id)
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

async function cancelDraft() {
  if (!draft.value) return
  saving.value = true
  error.value = ''
  try {
    await speakersApi.cancelDraft(speakerKey.value, draft.value.id, draft.value.revision)
    draft.value = undefined
    await load()
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    saving.value = false
  }
}

function releasePreview() {
  if (previewUrl.value) URL.revokeObjectURL(previewUrl.value)
  previewUrl.value = ''
}

async function startRecording() {
  if (!canRecord.value) return
  sampleError.value = ''
  releasePreview()
  await recorder.start(enrollmentLimits.value, () => void stopRecording())
  if (recorder.error.value) sampleError.value = formatApiError(new Error(recorder.error.value))
}

async function stopRecording() {
  const wav = await recorder.stop()
  if (!wav || !draft.value) return
  releasePreview()
  previewUrl.value = URL.createObjectURL(wav)
  uploading.value = true
  sampleError.value = ''
  uploadAbort?.abort()
  const controller = new AbortController()
  uploadAbort = controller
  try {
    draft.value = await speakersApi.uploadSample(
      speakerKey.value,
      draft.value.id,
      nextSlot.value,
      wav,
      draft.value.revision,
      controller.signal,
    )
  } catch (cause) {
    if (!controller.signal.aborted) {
      sampleError.value = formatApiError(cause)
      await refreshDraft()
    }
  } finally {
    if (uploadAbort === controller) uploadAbort = undefined
    uploading.value = false
  }
}

/** Re-read the draft after a lost response so the next request carries the true revision. */
async function refreshDraft() {
  if (!draft.value) return
  try {
    draft.value = await speakersApi.getDraft(speakerKey.value, draft.value.id)
  } catch {
    // Keep the last known draft; the user can retry or cancel.
  }
}

async function removeSample(slot: number) {
  if (!draft.value) return
  uploading.value = true
  sampleError.value = ''
  try {
    draft.value = await speakersApi.deleteSample(
      speakerKey.value,
      draft.value.id,
      slot,
      draft.value.revision,
    )
    releasePreview()
  } catch (cause) {
    sampleError.value = formatApiError(cause)
    await refreshDraft()
  } finally {
    uploading.value = false
  }
}

async function startHoldout() {
  if (!canValidate.value) return
  sampleError.value = ''
  validationError.value = ''
  await holdoutRecorder.start(enrollmentLimits.value, () => void stopHoldout())
  if (holdoutRecorder.error.value) {
    validationError.value = formatApiError(new Error(holdoutRecorder.error.value))
  }
}

async function stopHoldout() {
  const wav = await holdoutRecorder.stop()
  if (!wav || !draft.value) return
  validating.value = true
  validationError.value = ''
  try {
    const result = await speakersApi.validateHoldout(
      speakerKey.value,
      draft.value.id,
      wav,
      draft.value.revision,
    )
    draft.value = result.enrollment
  } catch (cause) {
    validationError.value = formatApiError(cause)
    await refreshDraft()
  } finally {
    validating.value = false
  }
}

async function finalizeDraft() {
  if (!draft.value || !speaker.value) return
  finalizing.value = true
  validationError.value = ''
  try {
    const result = await speakersApi.finalizeDraft(
      speakerKey.value,
      draft.value.id,
      speaker.value.revision,
      draft.value.revision,
    )
    speaker.value = result.speaker
    draft.value = undefined
    enrollOpen.value = false
  } catch (cause) {
    validationError.value = formatApiError(cause)
    await refreshDraft()
  } finally {
    finalizing.value = false
  }
}

function closeEnroll() {
  uploadAbort?.abort()
  uploadAbort = undefined
  recorder.dispose()
  holdoutRecorder.dispose()
  releasePreview()
  enrollOpen.value = false
  draft.value = undefined
  void load()
}

onBeforeUnmount(() => {
  uploadAbort?.abort()
  recorder.dispose()
  holdoutRecorder.dispose()
  releasePreview()
})

onMounted(load)
</script>

<template>
  <section class="space-y-6">
    <button class="inline-flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground" @click="router.push({ name: 'speakers' })">
      <ArrowLeft class="size-4" />
      {{ t('nav.speakers') }}
    </button>

    <PageHeader
      v-if="speaker"
      :eyebrow="t('speakers.eyebrow')"
      :title="speaker.name"
      :description="speaker.description ?? undefined"
    >
      <template #actions>
        <Button variant="outline" @click="load"><RefreshCw class="size-4" />{{ t('common.refresh') }}</Button>
        <Button variant="outline" @click="openEdit">{{ t('common.edit') }}</Button>
        <Button variant="outline" @click="deleteOpen = true"><Trash2 class="size-4" />{{ t('common.delete') }}</Button>
        <Button @click="openEnroll"><Play class="size-4" />{{ t('speakers.enroll') }}</Button>
      </template>
    </PageHeader>

    <p v-if="error" class="rounded-md border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive">
      {{ error }}
    </p>

    <div v-if="loading" class="py-10 text-center text-sm text-muted-foreground">{{ t('common.loading') }}</div>

    <template v-else-if="speaker">
      <div class="grid gap-4 md:grid-cols-3">
        <div class="rounded-lg border p-4">
          <p class="text-xs uppercase tracking-wide text-muted-foreground">{{ t('speakers.key') }}</p>
          <p class="mt-1 font-mono text-sm">{{ speaker.key }}</p>
        </div>
        <div class="rounded-lg border p-4">
          <p class="text-xs uppercase tracking-wide text-muted-foreground">{{ t('speakers.revision') }}</p>
          <p class="mt-1 text-sm">{{ speaker.revision }}</p>
        </div>
        <div class="rounded-lg border p-4">
          <p class="text-xs uppercase tracking-wide text-muted-foreground">{{ t('speakers.enabled') }}</p>
          <p class="mt-1 text-sm">{{ speaker.enabled ? t('speakers.enabled') : t('speakers.disabled') }}</p>
        </div>
      </div>

      <div class="rounded-lg border">
        <header class="border-b px-4 py-3 text-sm font-medium">{{ t('speakers.voiceprints') }}</header>
        <p v-if="speaker.voiceprints.length === 0" class="px-4 py-6 text-sm text-muted-foreground">
          {{ t('speakers.noVoiceprints') }}
        </p>
        <ul v-else class="divide-y">
          <li v-for="voiceprint in speaker.voiceprints" :key="voiceprint.embedding_space_id" class="grid gap-2 px-4 py-3 text-sm md:grid-cols-4">
            <span class="font-mono text-xs">{{ voiceprint.embedding_space_id }}</span>
            <span>{{ t('speakers.provider') }}: {{ voiceprint.enrolled_with_provider_key }}@{{ voiceprint.enrolled_with_provider_revision }}</span>
            <span>{{ t('speakers.sampleCount') }}: {{ voiceprint.sample_count }}</span>
            <span class="text-muted-foreground">
              {{ voiceprint.browser_validation_status === 'passed' ? t('speakers.validationPassed') : t('speakers.validation') }}
            </span>
          </li>
        </ul>
      </div>

      <div class="rounded-lg border">
        <header class="border-b px-4 py-3 text-sm font-medium">{{ t('speakers.drafts') }}</header>
        <p v-if="speaker.enrollment_drafts.length === 0" class="px-4 py-6 text-sm text-muted-foreground">
          {{ t('speakers.noDrafts') }}
        </p>
        <ul v-else class="divide-y">
          <li v-for="open in speaker.enrollment_drafts" :key="open.id" class="flex items-center justify-between px-4 py-3 text-sm">
            <span class="font-mono text-xs">{{ open.id }}</span>
            <span class="text-muted-foreground">{{ t('speakers.expiresAt') }}: {{ formatDateTime(new Date(open.expires_at * 1000)) }}</span>
            <Button variant="ghost" size="sm" @click="openEnroll(); resumeDraft(open.id)">
              {{ t('speakers.resumeDraft') }}
            </Button>
          </li>
        </ul>
      </div>
    </template>

    <BaseModal v-model="editOpen" :title="t('speakers.editTitle')">
      <form class="space-y-4" @submit.prevent="submitEdit">
        <label class="block space-y-1">
          <span class="text-sm font-medium">{{ t('speakers.name') }}</span>
          <input v-model="editForm.name" class="admin-input" required maxlength="128" />
        </label>
        <label class="block space-y-1">
          <span class="text-sm font-medium">{{ t('speakers.descriptionField') }}</span>
          <textarea v-model="editForm.description" class="admin-input" rows="3" maxlength="2048" />
        </label>
        <label class="flex items-center gap-2 text-sm">
          <input v-model="editForm.enabled" type="checkbox" />
          {{ t('speakers.enabled') }}
        </label>
      </form>
      <template #footer>
        <div class="flex justify-end gap-2">
          <Button variant="outline" @click="editOpen = false">{{ t('common.cancel') }}</Button>
          <Button :disabled="saving" @click="submitEdit">{{ t('common.saveChanges') }}</Button>
        </div>
      </template>
    </BaseModal>

    <BaseModal :model-value="enrollOpen" :title="t('speakers.enrollTitle')" @update:model-value="closeEnroll">
      <div class="space-y-4">
        <p v-if="providers.length === 0" class="text-sm text-muted-foreground">{{ t('speakers.noProviders') }}</p>

        <template v-else-if="!draft">
          <label class="block space-y-1">
            <span class="text-sm font-medium">{{ t('speakers.selectProvider') }}</span>
            <select v-model="selectedProviderKey" class="admin-input">
              <option value="" disabled>{{ t('speakers.selectProvider') }}</option>
              <option v-for="provider in providers" :key="provider.key" :value="provider.key">
                {{ provider.name }} ({{ provider.key }}@{{ provider.revision }})
              </option>
            </select>
          </label>
        </template>

        <div v-else class="space-y-4 rounded-md border p-4 text-sm">
          <div class="flex items-center gap-2 font-medium">
            <Mic class="size-4" />
            {{ t('speakers.draftStatus') }}: {{ draft.status }}
          </div>
          <p><span class="text-muted-foreground">{{ t('speakers.provider') }}:</span> {{ draft.provider_key }}@{{ draft.desired_provider_revision }}</p>
          <p class="break-all"><span class="text-muted-foreground">{{ t('speakers.embeddingSpace') }}:</span> <span class="font-mono text-xs">{{ draft.embedding_space_id }}</span></p>
          <p><span class="text-muted-foreground">{{ t('speakers.expiresAt') }}:</span> {{ formatDateTime(new Date(draft.expires_at * 1000)) }}</p>

          <div class="space-y-2 rounded-md bg-muted/40 p-3">
            <p class="font-medium">{{ t('speakers.samplesTitle', { count: draft.samples.length, max: maxSamples }) }}</p>
            <p v-if="draft.samples.length === 0" class="text-xs text-muted-foreground">{{ t('speakers.samplesEmpty') }}</p>
            <ul v-else class="space-y-1">
              <li v-for="sample in draft.samples" :key="sample.slot" class="flex items-center justify-between gap-2">
                <span>{{ t('speakers.sampleSlot', { slot: sample.slot }) }} · {{ (sample.duration_ms / 1000).toFixed(1) }}s</span>
                <Button variant="ghost" size="sm" :disabled="uploading" @click="removeSample(sample.slot)">
                  <Trash2 class="size-4" />
                </Button>
              </li>
            </ul>

            <p v-if="recorder.recording.value" class="text-xs text-muted-foreground">
              {{ t('speakers.recordingHint', { seconds: (recorder.elapsedMs.value / 1000).toFixed(1) }) }}
            </p>
            <p v-else class="text-xs text-muted-foreground">
              {{ t('speakers.recordPrompt', { min: enrollmentLimits.minClipMs / 1000, max: enrollmentLimits.maxClipMs / 1000 }) }}
            </p>
            <p v-if="sampleError" class="text-xs text-destructive">{{ sampleError }}</p>
            <div class="flex items-center gap-2">
              <Button v-if="recorder.recording.value" variant="outline" size="sm" @click="stopRecording">
                <Square class="size-4" />{{ t('speakers.stopRecording') }}
              </Button>
              <Button v-else :disabled="!canRecord" size="sm" @click="startRecording">
                <Mic class="size-4" />{{ uploading ? t('speakers.uploading') : t('speakers.recordSample') }}
              </Button>
              <audio v-if="previewUrl" :src="previewUrl" controls class="h-8" />
            </div>
          </div>

          <div class="space-y-2 rounded-md border p-3">
            <p class="font-medium">{{ t('speakers.validation') }}</p>
            <p class="text-xs" :class="draft.validation.valid_for_current_revision ? 'text-emerald-600' : 'text-muted-foreground'">
              {{ validationLabel }}
            </p>
            <p v-if="draft.samples.length < minSamples" class="text-xs text-muted-foreground">
              {{ t('speakers.holdoutPrompt') }}
            </p>
            <p v-if="holdoutRecorder.recording.value" class="text-xs text-muted-foreground">
              {{ t('speakers.recordingHint', { seconds: (holdoutRecorder.elapsedMs.value / 1000).toFixed(1) }) }}
            </p>
            <p v-if="validationError" class="text-xs text-destructive">{{ validationError }}</p>
            <div class="flex flex-wrap items-center gap-2">
              <Button v-if="holdoutRecorder.recording.value" variant="outline" size="sm" @click="stopHoldout">
                <Square class="size-4" />{{ t('speakers.stopRecording') }}
              </Button>
              <Button v-else :disabled="!canValidate" size="sm" @click="startHoldout">
                <Mic class="size-4" />{{ validating ? t('speakers.uploading') : t('speakers.validateHoldout') }}
              </Button>
              <Button :disabled="!canFinalize" size="sm" @click="finalizeDraft">
                {{ finalizing ? t('speakers.uploading') : t('speakers.finalize') }}
              </Button>
            </div>
            <p class="text-xs text-muted-foreground">{{ t('speakers.finalizeHint') }}</p>
          </div>
        </div>
      </div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <Button variant="outline" @click="closeEnroll">{{ t('common.close') }}</Button>
          <Button v-if="draft" variant="outline" :disabled="saving" @click="cancelDraft">
            <X class="size-4" />{{ t('speakers.cancelDraft') }}
          </Button>
          <Button v-else :disabled="saving || !selectedProviderKey" @click="startDraft">
            {{ t('speakers.startDraft') }}
          </Button>
        </div>
      </template>
    </BaseModal>

    <ConfirmDialog
      v-model="deleteOpen"
      :title="t('speakers.deleteTitle', { name: speaker?.name ?? '' })"
      :description="t('speakers.deleteDescription')"
      :confirm-label="t('speakers.deleteSubmit')"
      tone="danger"
      @confirm="confirmDelete"
    />
  </section>
</template>

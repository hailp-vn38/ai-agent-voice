<script setup lang="ts">
import { ArrowLeft, Eraser, Play, RefreshCw, Trash2 } from '@lucide/vue'
import { computed, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { formatApiError } from '@/api/errors'
import { speakersApi } from '@/api/speakers'
import type { Speaker } from '@/api/types/speakers'
import BaseModal from '@/components/admin/BaseModal.vue'
import ConfirmDialog from '@/components/admin/ConfirmDialog.vue'
import PageHeader from '@/components/admin/PageHeader.vue'
import SpeakerEnrollmentWizard from '@/components/speakers/SpeakerEnrollmentWizard.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const route = useRoute()
const router = useRouter()
const { t, formatDateTime } = useI18n()
const speakerKey = computed(() => String(route.params.speakerKey))
const speaker = ref<Speaker>()
const currentSpace = ref<string | null>(null)
const engineAvailable = ref(false)
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const editOpen = ref(false)
const deleteOpen = ref(false)
const purgeOpen = ref(false)
const enrollOpen = ref(false)
const editForm = ref({ name: '', description: '', enabled: true })

async function load() {
  loading.value = true; error.value = ''
  try {
    const [profile, summary] = await Promise.all([speakersApi.get(speakerKey.value), speakersApi.summary()])
    speaker.value = profile
    currentSpace.value = summary.embedding_space_id
    engineAvailable.value = summary.available
  } catch (cause) { error.value = formatApiError(cause) } finally { loading.value = false }
}
function openEdit() { if (speaker.value) { editForm.value = { name: speaker.value.name, description: speaker.value.description ?? '', enabled: speaker.value.enabled }; editOpen.value = true } }
async function saveEdit() {
  if (!speaker.value) return
  saving.value = true
  try { speaker.value = await speakersApi.update(speaker.value.key, { name: editForm.value.name.trim(), description: editForm.value.description.trim() || null, enabled: editForm.value.enabled }, speaker.value.revision); editOpen.value = false } catch (cause) { error.value = formatApiError(cause) } finally { saving.value = false }
}
async function deleteSpeaker() {
  if (!speaker.value) return
  saving.value = true
  try { await speakersApi.remove(speaker.value.key, speaker.value.revision); await router.push({ name: 'speakers' }) } catch (cause) { error.value = formatApiError(cause) } finally { saving.value = false }
}
async function purgeVoiceprint() {
  if (!speaker.value) return
  saving.value = true
  try { speaker.value = await speakersApi.purgeVoiceprint(speaker.value.key, speaker.value.revision); purgeOpen.value = false } catch (cause) { error.value = formatApiError(cause) } finally { saving.value = false }
}
function enrollmentCompleted(updated: Speaker) { speaker.value = updated; enrollOpen.value = false; void load() }
function voiceprintLabel(space: string) {
  if (!engineAvailable.value) return 'Không xác định khả năng tương thích'
  return space === currentSpace.value ? 'Đã đăng ký' : 'Cần thu lại mẫu'
}
onMounted(load)
</script>

<template>
  <section class="space-y-6">
    <button class="inline-flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground" @click="router.push({ name: 'speakers' })"><ArrowLeft class="size-4" />{{ t('nav.speakers') }}</button>
    <PageHeader v-if="speaker" :eyebrow="t('speakers.eyebrow')" :title="speaker.name" :description="speaker.description ?? undefined"><template #actions><Button variant="outline" @click="load"><RefreshCw class="size-4" />{{ t('common.refresh') }}</Button><Button variant="outline" @click="openEdit">{{ t('common.edit') }}</Button><Button v-if="speaker.voiceprints.length" variant="outline" @click="purgeOpen = true"><Eraser class="size-4" />{{ t('speakers.purge') }}</Button><Button variant="outline" @click="deleteOpen = true"><Trash2 class="size-4" />{{ t('common.delete') }}</Button><Button @click="enrollOpen = true"><Play class="size-4" />{{ speaker.voiceprints.length ? 'Thu lại giọng nói' : 'Đăng ký giọng nói' }}</Button></template></PageHeader>
    <p v-if="error" role="alert" class="rounded-md border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive">{{ error }}</p>
    <div v-if="loading" class="py-10 text-center text-sm text-muted-foreground">{{ t('common.loading') }}</div>
    <template v-else-if="speaker">
      <div class="grid gap-4 md:grid-cols-3"><div class="rounded-lg border p-4"><p class="text-xs uppercase text-muted-foreground">{{ t('speakers.key') }}</p><p class="mt-1 font-mono text-sm">{{ speaker.key }}</p></div><div class="rounded-lg border p-4"><p class="text-xs uppercase text-muted-foreground">{{ t('speakers.revision') }}</p><p class="mt-1 text-sm">{{ speaker.revision }}</p></div><div class="rounded-lg border p-4"><p class="text-xs uppercase text-muted-foreground">{{ t('speakers.enabled') }}</p><p class="mt-1 text-sm">{{ speaker.enabled ? t('speakers.enabled') : t('speakers.disabled') }}</p></div></div>
      <div class="rounded-lg border">
        <header class="border-b px-4 py-3 text-sm font-medium">{{ t('speakers.voiceprints') }}</header>
        <p v-if="!speaker.voiceprints.length" class="px-4 py-6 text-sm text-muted-foreground">Chưa có mẫu giọng nói.</p>
        <ul v-else class="divide-y">
          <li v-for="voiceprint in speaker.voiceprints" :key="voiceprint.embedding_space_id"
            class="grid gap-2 px-4 py-3 text-sm md:grid-cols-3">
            <span class="font-mono text-xs">{{ voiceprint.embedding_space_id }}</span>
            <span>{{ voiceprintLabel(voiceprint.embedding_space_id) }}</span>
            <span class="text-muted-foreground">{{ formatDateTime(new Date(voiceprint.enrolled_at * 1000)) }}</span>
          </li>
        </ul>
      </div>
    </template>
    <SpeakerEnrollmentWizard v-model:open="enrollOpen" :speaker-key="speakerKey" @completed="enrollmentCompleted" />
    <BaseModal v-model="editOpen" :title="t('speakers.editTitle')"><form class="space-y-4" @submit.prevent="saveEdit"><label class="block space-y-1"><span class="text-sm font-medium">{{ t('speakers.name') }}</span><input v-model="editForm.name" class="admin-input" required maxlength="128" /></label><label class="block space-y-1"><span class="text-sm font-medium">{{ t('speakers.descriptionField') }}</span><textarea v-model="editForm.description" class="admin-input" rows="3" maxlength="2048" /></label><label class="flex items-center gap-2 text-sm"><input v-model="editForm.enabled" type="checkbox" />{{ t('speakers.enabled') }}</label></form><template #footer><div class="flex justify-end gap-2"><Button variant="outline" @click="editOpen = false">{{ t('common.cancel') }}</Button><Button :disabled="saving" @click="saveEdit">{{ t('common.saveChanges') }}</Button></div></template></BaseModal>
    <ConfirmDialog v-model="deleteOpen" :title="t('speakers.deleteTitle', { name: speaker?.name ?? '' })" :description="t('speakers.deleteDescription')" :confirm-label="t('speakers.deleteSubmit')" tone="danger" @confirm="deleteSpeaker" />
    <ConfirmDialog v-model="purgeOpen" :title="t('speakers.purgeTitle', { name: speaker?.name ?? '' })" :description="t('speakers.purgeDescription')" :confirm-label="t('speakers.purgeSubmit')" tone="danger" @confirm="purgeVoiceprint" />
  </section>
</template>

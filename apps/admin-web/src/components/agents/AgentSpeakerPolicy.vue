<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { agentsApi } from '@/api/agents'
import { formatApiError, isApiError } from '@/api/errors'
import { speakersApi } from '@/api/speakers'
import type {
  AgentSpeakerBinding, AgentSpeakerPolicy, AgentSpeakerPolicyMode,
} from '@/api/types/speaker-policy'
import type { SpeakerSummary } from '@/api/types/speakers'
import BaseModal from '@/components/admin/BaseModal.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{ agentId: string }>()
const { t } = useI18n()
const policy = ref<AgentSpeakerPolicy>()
const bindings = ref<AgentSpeakerBinding[]>([])
const speakers = ref<SpeakerSummary[]>([])
const agentRevision = ref(0)
const loading = ref(false)
const busy = ref(false)
const error = ref('')
const dialogOpen = ref(false)
const draftSpeakerKey = ref('')
const modes: AgentSpeakerPolicyMode[] = ['off', 'observe']
const selectedSpeaker = computed(() => speakers.value.find((item) => item.key === draftSpeakerKey.value))
const availableSpeakers = computed(() =>
  speakers.value.filter((item) => !bindings.value.some((binding) => binding.speaker_key === item.key)),
)
const displayName = (key: string) => speakers.value.find((item) => item.key === key)?.name ?? key

async function load() {
  loading.value = true
  try {
    const [policyData, bindingPage, speakerPage] = await Promise.all([
      agentsApi.speakerPolicy(props.agentId),
      agentsApi.agentSpeakers(props.agentId),
      speakersApi.list({ page: 1, pageSize: 100 }),
    ])
    policy.value = policyData
    bindings.value = bindingPage.items
    agentRevision.value = bindingPage.agent_revision
    speakers.value = speakerPage.items
    error.value = ''
  } catch (cause) {
    error.value = formatApiError(cause)
  } finally {
    loading.value = false
  }
}
async function run(action: () => Promise<unknown>) {
  busy.value = true
  try {
    await action()
    await load()
  } catch (cause) {
    if (isApiError(cause) && cause.code === 'revision_conflict') {
      await load()
      error.value = t('agentSpeakerPolicy.revisionConflict')
    } else {
      error.value = formatApiError(cause)
    }
  } finally {
    busy.value = false
  }
}
function setMode(mode: AgentSpeakerPolicyMode) {
  if (!policy.value || policy.value.mode === mode) return
  void run(() => agentsApi.setSpeakerPolicy(props.agentId, mode, policy.value!.revision))
}
function saveBinding() {
  if (!draftSpeakerKey.value) return
  const key = draftSpeakerKey.value
  dialogOpen.value = false
  draftSpeakerKey.value = ''
  void run(() => agentsApi.setAgentSpeaker(props.agentId, key, agentRevision.value))
}
function removeBinding(binding: AgentSpeakerBinding) {
  void run(() => agentsApi.unlinkAgentSpeaker(props.agentId, binding.speaker_key, agentRevision.value))
}
watch(() => props.agentId, load, { immediate: true })
</script>

<template>
  <section class="space-y-4" aria-labelledby="agent-speaker-policy-title">
    <header class="flex items-start justify-between gap-4">
      <div>
        <h2 id="agent-speaker-policy-title" class="text-lg font-semibold">
          {{ t('agentSpeakerPolicy.title') }}
        </h2>
        <p class="text-sm text-muted-foreground">Nhận dạng người nói để cá nhân hóa hội thoại; không dùng để xác thực.</p>
      </div>
      <Button size="sm" variant="outline" :disabled="loading || busy"
        data-testid="add-grant" @click="dialogOpen = true">
        Thêm người nói
      </Button>
    </header>
    <p v-if="error" role="alert" class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm">{{ error }}</p>
    <div v-if="policy" class="space-y-2 rounded-md border border-border p-3">
      <p class="text-sm font-medium">Speaker Recognition</p>
      <div class="flex flex-wrap items-center gap-2">
        <Button v-for="mode in modes" :key="mode" size="sm"
          :variant="policy.mode === mode ? 'default' : 'outline'"
          :disabled="busy" :data-testid="`speaker-policy-mode-${mode}`"
          @click="setMode(mode)">
          {{ mode === 'observe' ? 'Bật nhận dạng' : 'Tắt' }}
        </Button>
        <Badge variant="secondary" data-testid="speaker-policy-current">
          {{ policy.mode === 'observe' ? 'Identification ON' : 'OFF' }}
        </Badge>
      </div>
      <p class="text-xs text-muted-foreground">Các thay đổi áp dụng khi thiết bị kết nối lại.</p>
    </div>
    <div class="space-y-2">
      <h3 class="text-sm font-medium">Người nói thuộc Agent</h3>
      <p v-if="!bindings.length && !loading" class="text-sm text-muted-foreground" data-testid="speaker-grants-empty">
        Chưa liên kết người nói.
      </p>
      <ul class="space-y-2">
        <li v-for="binding in bindings" :key="binding.speaker_key"
          class="flex items-center justify-between gap-2 rounded-md border border-border p-3"
          :data-testid="`speaker-binding-${binding.speaker_key}`">
          <div>
            <p class="text-sm font-medium">{{ displayName(binding.speaker_key) }}</p>
            <p class="font-mono text-xs text-muted-foreground">{{ binding.speaker_key }}</p>
          </div>
          <div class="flex items-center gap-2">
            <Badge v-if="!binding.usable" variant="danger" data-testid="speaker-binding-not-usable">Chưa có mẫu phù hợp</Badge>
            <Badge v-else variant="success">Sẵn sàng</Badge>
            <Button size="sm" variant="ghost" :disabled="busy" @click="removeBinding(binding)">
              Gỡ
            </Button>
          </div>
        </li>
      </ul>
    </div>
    <BaseModal v-model="dialogOpen" title="Thêm người nói vào Agent">
      <div class="space-y-4">
        <label class="block space-y-1 text-sm">
          <span class="font-medium">Người nói</span>
          <select v-model="draftSpeakerKey" class="admin-input" data-testid="grant-speaker-select">
            <option value="">Chọn người nói</option>
            <option v-for="item in availableSpeakers" :key="item.key" :value="item.key">
              {{ item.name }} ({{ item.key }})
            </option>
          </select>
        </label>
        <p v-if="selectedSpeaker && !selectedSpeaker.enabled" class="text-sm text-destructive"
          data-testid="grant-speaker-disabled">Người nói đang bị tắt.</p>
      </div>
      <template #footer>
        <div class="flex justify-end gap-2">
          <Button variant="outline" @click="dialogOpen = false">Hủy</Button>
          <Button :disabled="!draftSpeakerKey || busy" data-testid="grant-save" @click="saveBinding">
            Liên kết
          </Button>
        </div>
      </template>
    </BaseModal>
  </section>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import { agentsApi } from '@/api/agents'
import { formatApiError, isApiError } from '@/api/errors'
import { speakersApi } from '@/api/speakers'
import type { AgentTemplateLink } from '@/api/types/agents'
import type {
  AgentSpeakerBinding,
  AgentSpeakerPolicy,
  AgentSpeakerPolicyMode,
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
const agentRevision = ref(0)
const templates = ref<AgentTemplateLink[]>([])
const speakers = ref<SpeakerSummary[]>([])
const loading = ref(false)
const busy = ref(false)
const error = ref('')

const dialogOpen = ref(false)
const draftSpeakerKey = ref('')
const draftTemplateKeys = ref<string[]>([])

const modes: AgentSpeakerPolicyMode[] = ['off', 'observe', 'required']

const enabledTemplates = computed(() => templates.value.filter((template) => template.enabled !== false))
const selectedSpeaker = computed(() => speakers.value.find((speaker) => speaker.key === draftSpeakerKey.value))

async function load() {
  loading.value = true
  try {
    const [policyData, bindingData, templateData, speakerData] = await Promise.all([
      agentsApi.speakerPolicy(props.agentId),
      agentsApi.agentSpeakers(props.agentId),
      agentsApi.templates(props.agentId),
      speakersApi.list({ page: 1, pageSize: 100 }),
    ])
    policy.value = policyData
    bindings.value = bindingData.items
    agentRevision.value = bindingData.agent_revision
    templates.value = templateData.items
    speakers.value = speakerData.items
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

function blockerText(blocker: string) {
  return blocker === 'speaker_calibration_required'
    ? t('agentSpeakerPolicy.calibrationRequired')
    : blocker
}

function openGrant(speakerKey: string) {
  draftSpeakerKey.value = speakerKey
  dialogOpen.value = true
}

watch(draftSpeakerKey, (key) => {
  const existing = bindings.value.find((binding) => binding.speaker_key === key)
  draftTemplateKeys.value = existing ? [...existing.template_keys] : []
})

function toggleTemplate(key: string) {
  draftTemplateKeys.value = draftTemplateKeys.value.includes(key)
    ? draftTemplateKeys.value.filter((candidate) => candidate !== key)
    : [...draftTemplateKeys.value, key]
}

function saveGrant() {
  if (!draftSpeakerKey.value) return
  dialogOpen.value = false
  void run(() =>
    agentsApi.setAgentSpeaker(props.agentId, draftSpeakerKey.value, draftTemplateKeys.value, agentRevision.value),
  )
}

function removeTemplate(binding: AgentSpeakerBinding, templateKey: string) {
  const remaining = binding.template_keys.filter((key) => key !== templateKey)
  if (remaining.length === 0) {
    void run(() => agentsApi.unlinkAgentSpeaker(props.agentId, binding.speaker_key, agentRevision.value))
  } else {
    void run(() => agentsApi.setAgentSpeaker(props.agentId, binding.speaker_key, remaining, agentRevision.value))
  }
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
        <p class="text-sm text-muted-foreground">{{ t('agentSpeakerPolicy.description') }}</p>
      </div>
      <Button
        size="sm"
        variant="outline"
        :disabled="loading || busy"
        data-testid="add-grant"
        @click="openGrant('')"
      >
        {{ t('agentSpeakerPolicy.addGrant') }}
      </Button>
    </header>

    <p v-if="error" role="alert" class="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-sm">
      {{ error }}
    </p>

    <div v-if="policy" class="space-y-2 rounded-md border border-border p-3">
      <p class="text-sm font-medium">{{ t('agentSpeakerPolicy.modeLabel') }}</p>
      <div class="flex flex-wrap items-center gap-2">
        <Button
          v-for="mode in modes"
          :key="mode"
          size="sm"
          :variant="policy.mode === mode ? 'default' : 'outline'"
          :disabled="busy || (mode === 'required' && !policy.required_available)"
          :data-testid="`speaker-policy-mode-${mode}`"
          @click="setMode(mode)"
        >
          {{ t(`agentSpeakerPolicy.mode.${mode}`) }}
        </Button>
        <Badge variant="secondary" data-testid="speaker-policy-current">
          {{ t('agentSpeakerPolicy.currentMode', { mode: t(`agentSpeakerPolicy.mode.${policy.mode}`) }) }}
        </Badge>
      </div>
      <div v-if="!policy.required_available" class="text-sm text-muted-foreground" data-testid="required-blockers">
        <p>{{ t('agentSpeakerPolicy.requiredUnavailable') }}</p>
        <ul v-if="policy.required_blockers.length" class="ml-5 list-disc">
          <li v-for="blocker in policy.required_blockers" :key="blocker">{{ blockerText(blocker) }}</li>
        </ul>
      </div>
    </div>

    <div class="space-y-2">
      <h3 class="text-sm font-medium">{{ t('agentSpeakerPolicy.grants') }}</h3>
      <p v-if="!bindings.length && !loading" class="text-sm text-muted-foreground" data-testid="speaker-grants-empty">
        {{ t('agentSpeakerPolicy.empty') }}
      </p>
      <ul class="space-y-2">
        <li
          v-for="binding in bindings"
          :key="binding.speaker_key"
          class="rounded-md border border-border p-3"
          :data-testid="`speaker-binding-${binding.speaker_key}`"
        >
          <div class="flex items-center justify-between gap-2">
            <span class="font-mono text-sm">{{ binding.speaker_key }}</span>
            <div class="flex items-center gap-2">
              <Badge v-if="!binding.usable" variant="danger" data-testid="speaker-binding-not-usable">
                {{ t('agentSpeakerPolicy.notUsable') }}
              </Badge>
              <Badge v-else-if="binding.enabled" variant="success">{{ t('agentSpeakerPolicy.enabled') }}</Badge>
              <Badge v-else variant="secondary">{{ t('agentSpeakerPolicy.disabled') }}</Badge>
              <Button size="sm" variant="ghost" :disabled="busy" @click="removeBinding(binding)">
                {{ t('agentSpeakerPolicy.removeGrant') }}
              </Button>
            </div>
          </div>
          <div class="mt-2 flex flex-wrap gap-2">
            <span v-if="!binding.template_keys.length" class="text-sm text-muted-foreground">
              {{ t('agentSpeakerPolicy.noTemplates') }}
            </span>
            <span
              v-for="templateKey in binding.template_keys"
              :key="templateKey"
              class="inline-flex items-center gap-1 rounded-full border border-border px-2 py-0.5 text-xs"
            >
              {{ templateKey }}
              <button
                type="button"
                class="text-muted-foreground hover:text-foreground"
                :aria-label="t('agentSpeakerPolicy.removeTemplate', { template: templateKey })"
                :disabled="busy"
                @click="removeTemplate(binding, templateKey)"
              >
                ×
              </button>
            </span>
          </div>
        </li>
      </ul>
    </div>

    <BaseModal v-model="dialogOpen" :title="t('agentSpeakerPolicy.grantDialogTitle')">
      <div class="space-y-4">
        <label class="block space-y-1 text-sm">
          <span class="font-medium">{{ t('agentSpeakerPolicy.speaker') }}</span>
          <select v-model="draftSpeakerKey" class="w-full rounded-md border border-input bg-background px-2 py-1" data-testid="grant-speaker-select">
            <option value="">{{ t('agentSpeakerPolicy.selectSpeaker') }}</option>
            <option v-for="speaker in speakers" :key="speaker.key" :value="speaker.key">
              {{ speaker.name }} ({{ speaker.key }}){{ speaker.enabled ? '' : ' — disabled' }}
            </option>
          </select>
        </label>

        <fieldset class="space-y-1 text-sm">
          <legend class="font-medium">{{ t('agentSpeakerPolicy.templates') }}</legend>
          <p v-if="!enabledTemplates.length" class="text-muted-foreground">{{ t('agentSpeakerPolicy.noTemplates') }}</p>
          <label v-for="template in enabledTemplates" :key="template.key" class="flex items-center gap-2">
            <input
              type="checkbox"
              :checked="draftTemplateKeys.includes(template.key)"
              :disabled="busy"
              @change="toggleTemplate(template.key)"
            />
            <span>{{ template.name || template.key }}</span>
          </label>
        </fieldset>

        <p v-if="selectedSpeaker && !selectedSpeaker.enabled" class="text-sm text-danger-foreground" data-testid="grant-speaker-disabled">
          {{ t('agentSpeakerPolicy.speakerDisabled') }}
        </p>

        <div class="flex justify-end gap-2">
          <Button variant="outline" @click="dialogOpen = false">{{ t('agentSpeakerPolicy.cancel') }}</Button>
          <Button :disabled="!draftSpeakerKey || !draftTemplateKeys.length || busy" data-testid="grant-save" @click="saveGrant">
            {{ t('agentSpeakerPolicy.save') }}
          </Button>
        </div>
      </div>
    </BaseModal>
  </section>
</template>

<script setup lang="ts">
import { Link2 } from '@lucide/vue'
import { computed, ref, watch } from 'vue'

import BaseModal from '@/components/admin/BaseModal.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import type { Agent, AgentTemplate } from '@/domain/admin'

const props = defineProps<{
  /** Optional so a dialog opened without a selection renders instead of crashing. */
  template?: AgentTemplate
  agents: Agent[]
  /** Agents already linking the template, so the same edge is never created twice. */
  linkedAgentIds: string[]
}>()

const open = defineModel<boolean>({ required: true })
const emit = defineEmits<{ link: [agentId: string, setAsDefault: boolean] }>()

const { t } = useI18n()

const agentId = ref('')
const setAsDefault = ref(false)

const availableAgents = computed(() =>
  props.agents.filter((agent) => !props.linkedAgentIds.includes(agent.id)),
)

watch(
  () => [open.value, props.template?.id] as const,
  () => {
    if (!open.value) return
    agentId.value = availableAgents.value[0]?.id ?? ''
    setAsDefault.value = false
  },
  { immediate: true },
)

function link() {
  if (!props.template || !agentId.value) return
  emit('link', agentId.value, setAsDefault.value)
  open.value = false
}
</script>

<template>
  <BaseModal
    v-model="open"
    :title="t('linkTemplate.title', { name: template?.name ?? '' })"
    :description="t('linkTemplate.description')"
    width-class="max-w-md"
  >
    <form class="space-y-4" @submit.prevent="link">
      <label class="block space-y-1.5">
        <span class="text-sm font-medium">{{ t('linkTemplate.agent') }}</span>
        <select v-model="agentId" class="admin-input" required :disabled="availableAgents.length === 0">
          <option v-for="agent in availableAgents" :key="agent.id" :value="agent.id">
            {{ agent.name }}
          </option>
        </select>
      </label>

      <label class="flex cursor-pointer items-start gap-2.5 rounded-lg border border-border/70 px-3 py-2.5">
        <input v-model="setAsDefault" type="checkbox" class="mt-1 accent-foreground" />
        <span class="min-w-0">
          <span class="block text-sm font-medium">{{ t('linkTemplate.setDefault') }}</span>
          <span class="block text-xs text-muted-foreground">{{ t('linkTemplate.setDefaultHint') }}</span>
        </span>
      </label>

      <p v-if="availableAgents.length === 0" class="text-sm text-muted-foreground">
        {{ t('linkTemplate.allLinked') }}
      </p>

      <div class="flex justify-end gap-2 pt-1">
        <Button type="button" variant="outline" @click="open = false">{{ t('common.cancel') }}</Button>
        <Button type="submit" :disabled="availableAgents.length === 0 || !template">
          <Link2 class="size-4" />
          {{ t('linkTemplate.submit') }}
        </Button>
      </div>
    </form>
  </BaseModal>
</template>

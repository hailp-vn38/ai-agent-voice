<script setup lang="ts">
import {
  BookOpen,
  Bot,
  ChevronRight,
  Cpu,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  Workflow,
} from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { agentJobsApi } from '@/api/agent-jobs'
import { formatApiError, isApiError } from '@/api/errors'
import type { BackgroundAgentConfig, BackgroundSettings } from '@/api/types/agent-jobs'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'
import { useErrorToast, useSuccessToast } from '@/composables/useNotify'
import { isAgentCustomizedFromDefault } from './backgroundAgentUtils'
import SubagentCreateModal from './SubagentCreateModal.vue'

const router = useRouter()
const { t } = useI18n()

const settings = ref<BackgroundSettings | null>(null)
const search = ref('')
const loading = ref(true)
const saving = ref(false)
const error = ref('')
useErrorToast(() => error.value, { title: () => (unavailable.value ? t('jobUi.backendPending') : t('jobUi.loadFailed')), retry: () => load() })
const notice = ref('')
useSuccessToast(() => notice.value, { onDismiss: () => (notice.value = '') })
const unavailable = ref(false)

const subagentModalOpen = ref(false)
const subagentModalFlow = ref<'daily' | 'wiki'>('daily')

let controller: AbortController | undefined

const providerOptions = computed(() => settings.value?.llm_providers ?? [])
const agentsCatalog = computed(() => settings.value?.agents ?? [])
const skillsCatalog = computed(() => settings.value?.skills ?? [])

function sortFlowAgents(items: BackgroundAgentConfig[]): BackgroundAgentConfig[] {
  return [...items].sort((a, b) => {
    if (a.mode === 'primary' && b.mode !== 'primary') return -1
    if (a.mode !== 'primary' && b.mode === 'primary') return 1
    return 0
  })
}

function flowAgents(flowId: 'daily' | 'wiki'): BackgroundAgentConfig[] {
  const q = search.value.trim().toLowerCase()
  const all = sortFlowAgents(agentsCatalog.value.filter(a => a.flow_id === flowId))
  if (!q) return all
  return all.filter(
    a => a.id.toLowerCase().includes(q) || a.description.toLowerCase().includes(q),
  )
}

function totalFlowCount(flowId: 'daily' | 'wiki'): number {
  return agentsCatalog.value.filter(a => a.flow_id === flowId).length
}

function openCreateSubagentModal(flowId: 'daily' | 'wiki' = 'daily') {
  subagentModalFlow.value = flowId
  subagentModalOpen.value = true
}

function onSubagentCreated(updated: BackgroundSettings) {
  subagentModalOpen.value = false
  settings.value = {
    ...updated,
    llm_providers: updated.llm_providers ?? settings.value?.llm_providers ?? [],
    agents: updated.agents ?? settings.value?.agents ?? [],
    skills: updated.skills ?? settings.value?.skills ?? [],
  }
  notice.value = t('jobUi.subagentCreated')
}

async function deleteCustomSubagent(agent: BackgroundAgentConfig) {
  if (agent.is_builtin !== false || !settings.value || saving.value) return
  if (typeof window !== 'undefined' && typeof window.confirm === 'function') {
    const confirmed = window.confirm(t('jobUi.subagentDeleteConfirm', { id: agent.id }))
    if (!confirmed) return
  }
  saving.value = true
  error.value = ''
  notice.value = ''
  try {
    const updated = await agentJobsApi.deleteSubagent(agent.id, settings.value.revision)
    settings.value = {
      ...updated,
      llm_providers: updated.llm_providers ?? settings.value.llm_providers,
      agents: updated.agents ?? settings.value.agents,
      skills: updated.skills ?? settings.value.skills,
    }
    notice.value = t('jobUi.subagentDeleted')
  } catch (cause) {
    error.value = formatApiError(cause)
    if (isApiError(cause) && cause.status === 409) {
      await load()
    }
  } finally {
    saving.value = false
  }
}

function openConfigurePage(agent: BackgroundAgentConfig) {
  void router.push({ name: 'jobs-settings-agent', params: { agentId: agent.id } })
}

async function load() {
  controller?.abort()
  const pending = new AbortController()
  controller = pending
  loading.value = true
  error.value = ''
  notice.value = ''
  try {
    const config = await agentJobsApi.settings(pending.signal)
    if (pending.signal.aborted) return
    settings.value = config
    unavailable.value = false
  } catch (cause) {
    if (pending.signal.aborted) return
    unavailable.value = isApiError(cause) && cause.status === 404
    error.value = formatApiError(cause)
  } finally {
    if (controller === pending) loading.value = false
  }
}

onMounted(() => {
  void load()
})
onBeforeUnmount(() => controller?.abort())
</script>

<template>
  <section class="space-y-4">
    <DetailHeader
      :title="t('jobUi.agentsPageTitle')"
      :back-label="t('jobUi.allSettingsBack')"
      @back="router.push({ name: 'jobs-settings' })"
    >
      <template #icon>
        <div
          class="grid size-9 shrink-0 place-items-center rounded-xl bg-studio-violet/12 text-studio-violet"
        >
          <Bot class="size-4.5" aria-hidden="true" />
        </div>
      </template>
      <template #details>
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span class="font-semibold uppercase tracking-wider text-studio-violet">
            {{ t('jobUi.hubEyebrow') }}
          </span>
          <span aria-hidden="true">·</span>
          <span>{{ t('jobUi.agentsPageSubtitle') }}</span>
        </div>
      </template>
      <template #actions>
        <Button
          size="sm"
          :disabled="loading || saving"
          data-create-subagent-header
          @click="openCreateSubagentModal('daily')"
        >
          <Plus class="size-4" aria-hidden="true" />
          {{ t('jobUi.subagentAdd') }}
        </Button>
        <Button variant="outline" size="sm" :disabled="loading || saving" @click="load">
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" aria-hidden="true" />
          {{ t('jobUi.refresh') }}
        </Button>
      </template>
    </DetailHeader>

    <!-- Toolbar: Search + Actions -->
    <div class="flex flex-wrap items-center justify-between gap-2.5">
      <div class="relative min-w-[200px] flex-1 sm:max-w-xs">
        <Search
          class="pointer-events-none absolute left-3 top-2.5 size-4 text-muted-foreground"
          aria-hidden="true"
        />
        <input
          v-model="search"
          type="search"
          class="admin-input h-9 pl-9 text-xs"
          :placeholder="t('jobUi.agentsSearchPlaceholder')"
          :aria-label="t('jobUi.agentsSearchPlaceholder')"
          data-agent-search
        />
      </div>

      <div class="flex items-center gap-2">
        <Badge variant="secondary">{{ agentsCatalog.length }} agents</Badge>
      </div>
    </div>

    <div
      v-if="loading && !settings"
      class="studio-panel p-6 text-center text-sm text-muted-foreground"
      role="status"
    >
      {{ t('common.loading') }}
    </div>

    <template v-else-if="settings">
      <!-- Flow Sections (Daily Assistant & Knowledge Pipeline) -->
      <section
        v-for="flow in [
          {
            key: 'daily' as const,
            title: t('jobUi.flowDailyLabel'),
            fullTitle: t('jobUi.flowDailyTitle'),
            icon: Bot,
          },
          {
            key: 'wiki' as const,
            title: t('jobUi.flowWikiLabel'),
            fullTitle: t('jobUi.flowWikiTitle'),
            icon: BookOpen,
          },
        ]"
        :key="flow.key"
        v-show="totalFlowCount(flow.key) > 0 || flow.key === 'daily'"
        class="studio-panel overflow-hidden"
        :data-flow-section="flow.key"
      >
        <!-- Flow Block Header -->
        <div
          class="flex flex-wrap items-center justify-between gap-2.5 border-b border-border/70 bg-surface/70 px-4 py-2.5"
        >
          <div class="flex flex-wrap items-center gap-2">
            <component :is="flow.icon" class="size-4 text-studio-violet" aria-hidden="true" />
            <strong class="text-xs font-semibold text-foreground">{{ flow.title }}</strong>
            <span class="hidden text-xs text-muted-foreground md:inline">
              · {{ flow.fullTitle }}
            </span>
            <Badge variant="outline">{{ totalFlowCount(flow.key) }} agents</Badge>
          </div>

          <Button
            size="sm"
            variant="outline"
            type="button"
            class="h-7 px-2.5 text-xs"
            :disabled="saving"
            :data-subagent-create-btn="flow.key"
            @click="openCreateSubagentModal(flow.key)"
          >
            <Plus class="size-3.5" aria-hidden="true" />
            {{ t('jobUi.addShort') }}
          </Button>
        </div>

        <!-- Agent Rows -->
        <div v-if="flowAgents(flow.key).length > 0" class="divide-y divide-border/60">
          <div
            v-for="agent in flowAgents(flow.key)"
            :key="agent.id"
            class="flex items-center gap-2.5 px-4 py-2.5 transition-colors hover:bg-accent/30"
            :class="agent.mode === 'primary' ? 'bg-surface/40' : ''"
            :data-agent-row="agent.id"
          >
            <!-- Tree Indicator -->
            <div class="w-4 shrink-0 text-center text-xs text-muted-foreground">
              <Workflow v-if="agent.mode === 'primary'" class="mx-auto size-3.5" aria-hidden="true" />
              <span v-else aria-hidden="true">└</span>
            </div>

            <!-- Small Avatar -->
            <div
              class="grid size-6.5 shrink-0 place-items-center rounded-md border border-studio-violet/25 bg-studio-violet/10 text-studio-violet"
            >
              <Bot v-if="agent.mode === 'primary'" class="size-3.5" aria-hidden="true" />
              <Cpu v-else class="size-3.5" aria-hidden="true" />
            </div>

            <!-- Agent ID + Badges + Description -->
            <div class="min-w-0 flex-1">
              <div class="flex flex-wrap items-center gap-1.5">
                <button
                  type="button"
                  class="cursor-pointer font-mono text-xs font-semibold text-foreground hover:text-studio-violet"
                  @click="openConfigurePage(agent)"
                >
                  {{ agent.id }}
                </button>
                <Badge v-if="agent.mode === 'primary'" variant="default">
                  Root
                </Badge>
                <Badge v-if="agent.is_builtin === false" variant="outline">
                  {{ t('jobUi.subagentCustomBadge') }}
                </Badge>
                <Badge
                  v-if="agent.is_builtin !== false && isAgentCustomizedFromDefault(agent)"
                  variant="outline"
                >
                  {{ t('jobUi.customizedBadge') }}
                </Badge>
              </div>
              <p class="mt-0.5 truncate text-[11px] text-muted-foreground">
                {{ agent.description }}
              </p>
            </div>

            <!-- Steps -->
            <span class="hidden shrink-0 font-mono text-[11px] text-muted-foreground sm:inline">
              {{ agent.steps }} steps
            </span>

            <!-- Status Dot -->
            <span
              class="size-2 shrink-0 rounded-full"
              :class="agent.enabled ? 'bg-success' : 'bg-muted-foreground/45'"
              :title="agent.enabled ? t('jobUi.on') : t('jobUi.off')"
            />

            <!-- Delete Custom Subagent -->
            <Button
              v-if="agent.is_builtin === false"
              size="sm"
              variant="outline"
              type="button"
              class="h-7 border-danger/40 px-2 text-xs text-danger hover:bg-danger/10"
              :disabled="saving"
              :data-subagent-delete-card="agent.id"
              @click="deleteCustomSubagent(agent)"
            >
              <Trash2 class="size-3.5" aria-hidden="true" />
              <span class="hidden sm:inline">{{ t('common.delete') }}</span>
            </Button>

            <!-- Configure Button -->
            <Button
              size="sm"
              variant="outline"
              type="button"
              class="h-7 px-2.5 text-xs"
              :data-agent="agent.id"
              @click="openConfigurePage(agent)"
            >
              {{ t('jobUi.configureAgent') }}
              <ChevronRight class="size-3.5" aria-hidden="true" />
            </Button>
          </div>
        </div>

        <div v-else class="p-4 text-xs text-muted-foreground">
          {{ t('jobUi.agentsEmptySearch') }}
        </div>
      </section>
    </template>

    <SubagentCreateModal
      :open="subagentModalOpen"
      :initial-flow-id="subagentModalFlow"
      :revision="settings?.revision ?? 1"
      :providers="providerOptions"
      :skills="skillsCatalog"
      @close="subagentModalOpen = false"
      @created="onSubagentCreated"
    />
  </section>
</template>

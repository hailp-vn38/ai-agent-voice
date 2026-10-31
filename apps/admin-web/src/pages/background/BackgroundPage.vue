<script setup lang="ts">
import {
  Activity,
  ArrowRight,
  BookOpen,
  Bot,
  Cpu,
  FileText,
  RefreshCw,
  Settings2,
} from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { agentJobsApi } from '@/api/agent-jobs'
import { formatApiError, isApiError } from '@/api/errors'
import type { BackgroundSettings } from '@/api/types/agent-jobs'
import DetailHeader from '@/components/admin/DetailHeader.vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const router = useRouter()
const { t } = useI18n()

const settings = ref<BackgroundSettings | null>(null)
const loading = ref(true)
const error = ref('')
const unavailable = ref(false)

let controller: AbortController | undefined

const agentsCatalog = computed(() => settings.value?.agents ?? [])
const skillsCatalog = computed(() => settings.value?.skills ?? [])

const rootAgentsCount = computed(
  () => agentsCatalog.value.filter(a => a.mode === 'primary').length,
)
const subagentsCount = computed(
  () => agentsCatalog.value.filter(a => a.mode !== 'primary').length,
)
const customSkillsCount = computed(
  () => skillsCatalog.value.filter(s => !s.is_builtin).length,
)
const flowCount = computed(() => {
  const flows = new Set(agentsCatalog.value.map(a => a.flow_id))
  return flows.size || 2
})
const configuredDefaultsCount = computed(() => {
  if (!settings.value) return 0
  let count = 0
  if (settings.value.orchestrator_llm_provider) count++
  if (settings.value.worker_llm_provider) count++
  return count
})

const quickAgent = computed(() => {
  const catalog = agentsCatalog.value
  return (
    catalog.find(a => a.id === 'source_collector') ??
    catalog.find(a => a.id === 'wiki_researcher') ??
    catalog.find(a => a.mode !== 'primary') ??
    catalog[0] ?? {
      id: 'source_collector',
      flow_id: 'daily',
    }
  )
})

const quickSkill = computed(() => {
  const catalog = skillsCatalog.value
  return (
    catalog.find(s => s.id === 'wiki-query') ??
    catalog[0] ?? {
      id: 'wiki-query',
      is_builtin: true,
    }
  )
})

async function load() {
  controller?.abort()
  const pending = new AbortController()
  controller = pending
  loading.value = true
  error.value = ''
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
      :title="t('jobUi.hubTitle')"
      :back-label="t('jobUi.jobs')"
      @back="router.push({ name: 'jobs' })"
    >
      <template #icon>
        <div
          class="grid size-9 shrink-0 place-items-center rounded-xl bg-studio-violet/12 text-studio-violet"
        >
          <Settings2 class="size-4.5" aria-hidden="true" />
        </div>
      </template>
      <template #details>
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span class="font-semibold uppercase tracking-wider text-studio-violet">
            {{ t('jobUi.hubEyebrow') }}
          </span>
          <span aria-hidden="true">·</span>
          <span>{{ t('jobUi.hubSubtitle') }}</span>
        </div>
      </template>
      <template #actions>
        <Button variant="outline" size="sm" :disabled="loading" @click="load">
          <RefreshCw class="size-4" :class="{ 'animate-spin': loading }" aria-hidden="true" />
          {{ t('jobUi.refresh') }}
        </Button>
      </template>
    </DetailHeader>

    <div
      v-if="error"
      role="alert"
      class="rounded-lg border border-danger/30 bg-danger/5 p-3.5 text-sm text-danger"
    >
      <p class="font-medium">{{ unavailable ? t('jobUi.backendPending') : t('jobUi.loadFailed') }}</p>
      <p class="mt-1 break-words">{{ error }}</p>
    </div>

    <div
      v-if="loading && !settings"
      class="studio-panel p-6 text-center text-sm text-muted-foreground"
      role="status"
    >
      {{ t('common.loading') }}
    </div>

    <template v-else-if="settings">
      <!-- Feature Banner: Background Runtime -->
      <section
        class="flex flex-wrap items-center justify-between gap-4 rounded-2xl border border-studio-violet/30 bg-linear-to-br from-card via-card to-studio-violet/10 p-4 shadow-xs"
        data-hub-runtime-banner
      >
        <div class="flex items-start gap-3">
          <span
            class="grid size-9 shrink-0 place-items-center rounded-xl bg-studio-violet/15 text-studio-violet"
          >
            <Activity class="size-5" aria-hidden="true" />
          </span>
          <div>
            <h2 class="flex flex-wrap items-center gap-2 text-sm font-semibold">
              <span>{{ t('jobUi.hubRuntimeTitle') }}</span>
              <Badge :variant="settings.enabled ? 'success' : 'outline'">
                {{ settings.enabled ? t('jobUi.hubRuntimeActive') : t('jobUi.hubRuntimeInactive') }}
              </Badge>
            </h2>
            <p class="mt-0.5 text-xs text-muted-foreground">
              {{ t('jobUi.hubRuntimeHint') }}
            </p>
          </div>
        </div>

        <Button
          size="sm"
          data-page="runtime"
          data-hub-runtime-btn
          @click="router.push({ name: 'jobs-settings-runtime' })"
        >
          {{ t('jobUi.hubRuntimeAction') }}
          <ArrowRight class="size-3.5" aria-hidden="true" />
        </Button>
      </section>

      <!-- Configuration Categories -->
      <div class="space-y-3">
        <div class="flex flex-wrap items-center justify-between gap-2 pt-1">
          <div>
            <h2 class="text-base font-semibold tracking-tight">
              {{ t('jobUi.hubCategoriesTitle') }}
            </h2>
            <small class="text-xs text-muted-foreground">
              {{ t('jobUi.hubCategoriesSubtitle') }}
            </small>
          </div>

          <div class="flex flex-wrap items-center gap-1.5">
            <Badge variant="default">{{ rootAgentsCount }} Root Agents</Badge>
            <Badge variant="outline">{{ subagentsCount }} Subagents</Badge>
            <Badge variant="outline">{{ skillsCatalog.length }} Skills</Badge>
          </div>
        </div>

        <div class="grid gap-3.5 md:grid-cols-2">
          <!-- Tile 1: Agents & Subagents -->
          <div
            role="button"
            tabindex="0"
            data-page="agents"
            data-hub-tile="agents"
            class="group studio-panel cursor-pointer p-4 transition-all hover:-translate-y-0.5 hover:border-studio-violet/45 hover:shadow-md"
            @click="router.push({ name: 'jobs-settings-agents' })"
            @keydown.enter.prevent="router.push({ name: 'jobs-settings-agents' })"
            @keydown.space.prevent="router.push({ name: 'jobs-settings-agents' })"
          >
            <div class="flex items-start justify-between">
              <span
                class="grid size-9 place-items-center rounded-xl bg-studio-violet/12 text-studio-violet"
              >
                <Bot class="size-5" aria-hidden="true" />
              </span>
              <ArrowRight
                class="size-4 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-studio-violet"
                aria-hidden="true"
              />
            </div>
            <h3 class="mt-3 text-base font-semibold tracking-tight">
              {{ t('jobUi.hubTileAgentsTitle') }}
            </h3>
            <p class="mt-1 mb-3 max-w-md text-xs leading-relaxed text-muted-foreground">
              {{ t('jobUi.hubTileAgentsDesc') }}
            </p>
            <Badge variant="outline">
              {{ agentsCatalog.length }} agents · {{ flowCount }} flows
            </Badge>
          </div>

          <!-- Tile 2: Skill Library -->
          <div
            role="button"
            tabindex="0"
            data-page="skills"
            data-hub-tile="skills"
            class="group studio-panel cursor-pointer p-4 transition-all hover:-translate-y-0.5 hover:border-studio-violet/45 hover:shadow-md"
            @click="router.push({ name: 'jobs-settings-skills' })"
            @keydown.enter.prevent="router.push({ name: 'jobs-settings-skills' })"
            @keydown.space.prevent="router.push({ name: 'jobs-settings-skills' })"
          >
            <div class="flex items-start justify-between">
              <span
                class="grid size-9 place-items-center rounded-xl bg-studio-cyan/12 text-studio-cyan"
              >
                <BookOpen class="size-5" aria-hidden="true" />
              </span>
              <ArrowRight
                class="size-4 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-studio-violet"
                aria-hidden="true"
              />
            </div>
            <h3 class="mt-3 text-base font-semibold tracking-tight">
              {{ t('jobUi.hubTileSkillsTitle') }}
            </h3>
            <p class="mt-1 mb-3 max-w-md text-xs leading-relaxed text-muted-foreground">
              {{ t('jobUi.hubTileSkillsDesc') }}
            </p>
            <Badge variant="outline">
              {{ skillsCatalog.length }} skills · {{ customSkillsCount }} custom
            </Badge>
          </div>

          <!-- Tile 3: Runtime Settings -->
          <div
            role="button"
            tabindex="0"
            data-page="runtime"
            data-hub-tile="runtime"
            class="group studio-panel cursor-pointer p-4 transition-all hover:-translate-y-0.5 hover:border-studio-violet/45 hover:shadow-md"
            @click="router.push({ name: 'jobs-settings-runtime' })"
            @keydown.enter.prevent="router.push({ name: 'jobs-settings-runtime' })"
            @keydown.space.prevent="router.push({ name: 'jobs-settings-runtime' })"
          >
            <div class="flex items-start justify-between">
              <span class="grid size-9 place-items-center rounded-xl bg-success/12 text-success">
                <Activity class="size-5" aria-hidden="true" />
              </span>
              <ArrowRight
                class="size-4 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-studio-violet"
                aria-hidden="true"
              />
            </div>
            <h3 class="mt-3 text-base font-semibold tracking-tight">
              {{ t('jobUi.hubTileRuntimeTitle') }}
            </h3>
            <p class="mt-1 mb-3 max-w-md text-xs leading-relaxed text-muted-foreground">
              {{ t('jobUi.hubTileRuntimeDesc') }}
            </p>
            <Badge :variant="settings.enabled ? 'success' : 'outline'">
              {{ settings.enabled ? t('jobUi.on') : t('jobUi.off') }}
            </Badge>
          </div>

          <!-- Tile 4: LLM & Providers -->
          <div
            role="button"
            tabindex="0"
            data-page="providers"
            data-hub-tile="providers"
            class="group studio-panel cursor-pointer p-4 transition-all hover:-translate-y-0.5 hover:border-studio-violet/45 hover:shadow-md"
            @click="router.push({ name: 'jobs-settings-providers' })"
            @keydown.enter.prevent="router.push({ name: 'jobs-settings-providers' })"
            @keydown.space.prevent="router.push({ name: 'jobs-settings-providers' })"
          >
            <div class="flex items-start justify-between">
              <span
                class="grid size-9 place-items-center rounded-xl bg-warning/15 text-warning-foreground dark:text-warning"
              >
                <Cpu class="size-5" aria-hidden="true" />
              </span>
              <ArrowRight
                class="size-4 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-studio-violet"
                aria-hidden="true"
              />
            </div>
            <h3 class="mt-3 text-base font-semibold tracking-tight">
              {{ t('jobUi.hubTileProvidersTitle') }}
            </h3>
            <p class="mt-1 mb-3 max-w-md text-xs leading-relaxed text-muted-foreground">
              {{ t('jobUi.hubTileProvidersDesc') }}
            </p>
            <Badge variant="outline">
              {{ configuredDefaultsCount }} defaults configured
            </Badge>
          </div>
        </div>
      </div>

      <!-- Quick Access -->
      <div class="space-y-3 pt-1">
        <h2 class="text-base font-semibold tracking-tight">
          {{ t('jobUi.hubQuickAccessTitle') }}
        </h2>

        <div class="grid gap-3 md:grid-cols-3">
          <!-- Quick Agent -->
          <div
            role="button"
            tabindex="0"
            data-quick-agent
            class="studio-panel cursor-pointer p-3.5 transition-colors hover:border-studio-violet/45 hover:bg-accent/35"
            @click="router.push({ name: 'jobs-settings-agent', params: { agentId: quickAgent.id } })"
            @keydown.enter.prevent="
              router.push({ name: 'jobs-settings-agent', params: { agentId: quickAgent.id } })
            "
            @keydown.space.prevent="
              router.push({ name: 'jobs-settings-agent', params: { agentId: quickAgent.id } })
            "
          >
            <div class="mb-2 flex items-center gap-2">
              <Bot class="size-4 text-studio-violet" aria-hidden="true" />
              <strong class="font-mono text-xs">{{ quickAgent.id }}</strong>
            </div>
            <p class="mb-2.5 text-xs text-muted-foreground">
              {{ t('jobUi.hubQuickAgentDesc') }}
            </p>
            <Badge variant="outline">
              {{ quickAgent.flow_id === 'wiki' ? 'Wiki Flow' : 'Daily Flow' }}
            </Badge>
          </div>

          <!-- Quick Skill -->
          <div
            role="button"
            tabindex="0"
            data-quick-skill
            class="studio-panel cursor-pointer p-3.5 transition-colors hover:border-studio-violet/45 hover:bg-accent/35"
            @click="router.push({ name: 'jobs-settings-skill', params: { skillId: quickSkill.id } })"
            @keydown.enter.prevent="
              router.push({ name: 'jobs-settings-skill', params: { skillId: quickSkill.id } })
            "
            @keydown.space.prevent="
              router.push({ name: 'jobs-settings-skill', params: { skillId: quickSkill.id } })
            "
          >
            <div class="mb-2 flex items-center gap-2">
              <FileText class="size-4 text-studio-cyan" aria-hidden="true" />
              <strong class="font-mono text-xs">{{ quickSkill.id }}</strong>
            </div>
            <p class="mb-2.5 text-xs text-muted-foreground">
              {{ t('jobUi.hubQuickSkillDesc') }}
            </p>
            <Badge variant="outline">
              {{
                quickSkill.is_builtin
                  ? t('jobUi.skillBuiltinBadge')
                  : t('jobUi.skillCustomBadge')
              }}
            </Badge>
          </div>

          <!-- Quick Default Models -->
          <div
            role="button"
            tabindex="0"
            data-quick-providers
            class="studio-panel cursor-pointer p-3.5 transition-colors hover:border-studio-violet/45 hover:bg-accent/35"
            @click="router.push({ name: 'jobs-settings-providers' })"
            @keydown.enter.prevent="router.push({ name: 'jobs-settings-providers' })"
            @keydown.space.prevent="router.push({ name: 'jobs-settings-providers' })"
          >
            <div class="mb-2 flex items-center gap-2">
              <Cpu class="size-4 text-warning" aria-hidden="true" />
              <strong class="text-xs">{{ t('jobUi.hubQuickProvidersTitle') }}</strong>
            </div>
            <p class="mb-2.5 text-xs text-muted-foreground">
              {{ t('jobUi.hubQuickProvidersDesc') }}
            </p>
            <Badge variant="outline">{{ configuredDefaultsCount }} Providers</Badge>
          </div>
        </div>
      </div>
    </template>
  </section>
</template>

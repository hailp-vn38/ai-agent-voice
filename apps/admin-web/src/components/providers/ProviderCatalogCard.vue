<script setup lang="ts">
import { ArrowRight } from '@lucide/vue'
import { computed, ref } from 'vue'

import ProviderActionsMenu from '@/components/providers/ProviderActionsMenu.vue'
import ProviderUsageList from '@/components/providers/ProviderUsageList.vue'
import { Badge } from '@/components/ui/badge'
import { useI18n } from '@/composables/useI18n'
import { effectiveLlmTags, type ProviderInstance, type ProviderUsageEntry } from '@/domain/admin'
import { providerTypeIcons } from '@/lib/providerTypeIcons'

const props = defineProps<{
  provider: ProviderInstance
  /** Derived from the template bindings on every render, never persisted. */
  usage: ProviderUsageEntry[]
}>()

const emit = defineEmits<{
  open: []
  edit: []
  link: []
  duplicate: []
  delete: []
}>()

const { t, providerStatusLabel } = useI18n()

const usageOpen = ref(false)

const typeIcon = computed(() => providerTypeIcons[props.provider.type])
const llmTags = computed(() => effectiveLlmTags(props.provider))
const usageCount = computed(() => props.usage.length)

const typeIconBoxClass = computed(() => {
  switch (props.provider.type) {
    case 'asr':
      return 'border-sky-500/25 bg-sky-500/12 text-sky-400'
    case 'tts':
      return 'border-emerald-500/25 bg-emerald-500/12 text-emerald-400'
    case 'vad':
      return 'border-amber-500/25 bg-amber-500/12 text-amber-400'
    case 'llm':
      return 'border-violet-500/25 bg-violet-500/12 text-violet-400'
    default:
      return 'border-primary/25 bg-primary/12 text-primary'
  }
})

const statusChipClass = computed(() => {
  if (props.provider.status === 'error') {
    return 'border-danger/30 bg-danger/10 text-danger'
  }
  if (props.provider.status === 'disabled') {
    return 'border-border/70 bg-muted/40 text-muted-foreground'
  }
  return 'border-emerald-500/30 bg-emerald-500/10 text-emerald-400'
})

const statusDotClass = computed(() => {
  if (props.provider.status === 'error') return 'bg-danger'
  if (props.provider.status === 'disabled') return 'bg-muted-foreground'
  return 'bg-emerald-400 shadow-[0_0_0_3px_rgba(16,185,129,0.14)]'
})
</script>

<template>
  <article
    role="button"
    tabindex="0"
    class="studio-panel group flex h-full cursor-pointer flex-col justify-between gap-2.5 p-3.5 transition-all hover:-translate-y-0.5 hover:border-primary/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
    @click="emit('open')"
    @keydown.enter.prevent="emit('open')"
    @keydown.space.prevent="emit('open')"
  >
    <div class="space-y-2">
      <!-- Compact header: Type icon box + Provider name + Quick actions + Arrow -->
      <div class="flex items-center gap-2.5">
        <span
          class="flex size-8 shrink-0 items-center justify-center rounded-lg border"
          :class="typeIconBoxClass"
        >
          <component :is="typeIcon" class="size-4" aria-hidden="true" />
        </span>

        <h3 class="min-w-0 flex-1 truncate text-sm font-semibold tracking-tight text-foreground" :title="provider.name">
          {{ provider.name }}
        </h3>

        <div class="flex shrink-0 items-center gap-1" @click.stop>
          <ProviderActionsMenu
            :provider="provider"
            @view="emit('open')"
            @edit="emit('edit')"
            @link="emit('link')"
            @duplicate="emit('duplicate')"
            @delete="emit('delete')"
          />
        </div>

        <ArrowRight
          class="size-3.5 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-primary"
          aria-hidden="true"
        />
      </div>

      <!-- Middle row: Model + Status chip -->
      <div class="flex items-center justify-between gap-2">
        <p
          class="max-w-[65%] truncate font-mono text-xs text-muted-foreground"
          :title="provider.model || t('providers.modelMissing')"
        >
          {{ provider.model || t('providers.modelMissing') }}
        </p>

        <span
          class="inline-flex shrink-0 items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] font-medium"
          :class="statusChipClass"
        >
          <span class="size-1.5 rounded-full" :class="statusDotClass" aria-hidden="true" />
          {{ providerStatusLabel(provider.status) }}
        </span>
      </div>

      <!-- Optional LLM capability tags -->
      <div v-if="llmTags.length" class="flex flex-wrap gap-1 pt-0.5">
        <span
          v-for="tag in llmTags"
          :key="tag"
          data-llm-tag
          class="inline-flex items-center rounded-md border border-border/70 bg-muted/40 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
        >
          {{ tag }}
        </span>
      </div>
    </div>

    <!-- Compact footer: Adapter + Template usage + Config badge -->
    <div class="space-y-2 border-t border-border/60 pt-2.5">
      <div class="flex items-center justify-between gap-2 text-xs text-muted-foreground">
        <span class="truncate font-mono text-[11px]">{{ provider.adapter }}</span>

        <div class="flex shrink-0 items-center gap-1.5">
          <button
            type="button"
            class="rounded px-1 py-0.5 text-[11px] transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            :class="usageCount ? 'text-muted-foreground' : 'text-muted-foreground/70'"
            :aria-expanded="usageOpen"
            :aria-label="t('providers.usageToggleLabel', { name: provider.name })"
            :disabled="!usageCount"
            @click.stop="usageOpen = !usageOpen"
          >
            {{ t('providers.templatesShort', { count: usageCount }) }}
          </button>

          <Badge v-if="provider.source === 'config'" variant="outline" class="px-1.5 py-0 text-[10px]">
            Config
          </Badge>
        </div>
      </div>

      <ProviderUsageList
        v-if="usageOpen"
        :provider-name="provider.name"
        :usage="usage"
        @navigate="emit('open')"
      />
    </div>
  </article>
</template>

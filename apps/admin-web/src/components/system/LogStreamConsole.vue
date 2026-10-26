<script setup lang="ts">
import {
  ArrowDown,
  ChevronDown,
  ChevronRight,
  Download,
  Filter,
  Pause,
  Play,
  Sparkles,
  Trash2,
  X,
} from '@lucide/vue'
import { computed, nextTick, ref, watch } from 'vue'

import type {
  LogLevel,
  LogLevelSummary,
  LogQueryFilter,
  StructuredLogEntry,
} from '@/api/types/system-logs'
import { LOG_LEVELS } from '@/api/types/system-logs'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{
  entries: StructuredLogEntry[]
  counts: LogLevelSummary
  bufferedCount: number
  capacity: number
  droppedEvicted: number
  filter: LogQueryFilter
  paused: boolean
  queuedCount: number
  laggedCount: number
  demoMode: boolean
  loading: boolean
}>()

const emit = defineEmits<{
  'update:filter': [patch: Partial<LogQueryFilter>]
  'clear-correlation': []
  'toggle-pause': []
  'scroll-pause': []
  'clear-view': []
  'export-jsonl': []
  'simulate-demo': []
}>()

const { t } = useI18n()
const expandedSeqs = ref<Set<number>>(new Set())
const consoleContainer = ref<HTMLElement | null>(null)
let programmaticScroll = false

async function scrollToBottom() {
  await nextTick()
  const el = consoleContainer.value
  if (!el) return
  programmaticScroll = true
  el.scrollTop = el.scrollHeight
  programmaticScroll = false
}

watch(
  () => [props.entries.length, props.entries[props.entries.length - 1]?.seq] as const,
  () => {
    if (!props.paused) {
      void scrollToBottom()
    }
  },
  { immediate: true },
)

watch(
  () => props.paused,
  (isPaused, wasPaused) => {
    if (wasPaused && !isPaused) {
      void scrollToBottom()
    }
  },
)

const SUBSYSTEM_CHIPS = [
  { key: '', labelKey: 'system.logs.allSubsystems' as const },
  { key: 'voice.ws', label: 'voice.ws' },
  { key: 'voice.vad', label: 'voice.vad' },
  { key: 'voice.asr', label: 'voice.asr' },
  { key: 'voice.speaker', label: 'voice.speaker' },
  { key: 'voice.llm', label: 'voice.llm' },
  { key: 'voice.tts', label: 'voice.tts' },
  { key: 'mcp.device', label: 'mcp.device' },
  { key: 'mcp.external', label: 'mcp.external' },
  { key: 'task_flow', label: 'task_flow' },
  { key: 'notification', label: 'notification' },
  { key: 'admin.api', label: 'admin.api' },
  { key: 'system', label: 'system' },
]

const hasCorrelationFilters = computed(() =>
  Boolean(
    props.filter.session_id ||
      props.filter.turn_id != null ||
      props.filter.request_id ||
      props.filter.task_id,
  ),
)

function toggleRow(seq: number) {
  const next = new Set(expandedSeqs.value)
  if (next.has(seq)) {
    next.delete(seq)
  } else {
    next.add(seq)
  }
  expandedSeqs.value = next
}

function onConsoleScroll(event: Event) {
  if (programmaticScroll) return
  const el = event.target as HTMLElement | null
  if (!el || props.paused) return
  const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight
  if (distanceFromBottom > 48) {
    emit('scroll-pause')
  }
}

function formatTimeMs(ms: number): string {
  const date = new Date(ms)
  if (Number.isNaN(date.getTime())) return '—'
  const hh = String(date.getHours()).padStart(2, '0')
  const mm = String(date.getMinutes()).padStart(2, '0')
  const ss = String(date.getSeconds()).padStart(2, '0')
  const mmm = String(date.getMilliseconds()).padStart(3, '0')
  return `${hh}:${mm}:${ss}.${mmm}`
}

function levelVariant(level: LogLevel): 'danger' | 'secondary' | 'outline' | 'default' {
  if (level === 'ERROR') return 'danger'
  if (level === 'WARN') return 'default'
  if (level === 'INFO') return 'secondary'
  return 'outline'
}
</script>

<template>
  <div class="space-y-4">
    <!-- Summary bar -->
    <div class="flex flex-wrap items-center justify-between gap-3 rounded-xl border bg-card px-4 py-3 text-xs">
      <div class="flex flex-wrap items-center gap-2">
        <Badge
          :variant="paused ? 'secondary' : 'success'"
          data-testid="logs-stream-status"
        >
          {{ paused ? t('system.logs.paused') : t('system.logs.live') }}
        </Badge>
        <Badge v-if="demoMode" variant="outline" data-testid="logs-demo-badge">
          {{ t('notifications.demoBadge') }}
        </Badge>
        <span class="font-medium text-destructive">ERROR: {{ counts.error }}</span>
        <span class="font-medium text-amber-600 dark:text-amber-400">WARN: {{ counts.warn }}</span>
        <span class="text-muted-foreground">INFO: {{ counts.info }}</span>
        <span class="text-muted-foreground">DEBUG: {{ counts.debug }}</span>
      </div>
      <div class="flex flex-wrap items-center gap-3 text-muted-foreground">
        <span>
          {{ t('system.logs.bufferUsage', { buffered: bufferedCount, capacity }) }}
        </span>
        <span v-if="droppedEvicted > 0">
          {{ t('system.logs.evictedCount', { count: droppedEvicted }) }}
        </span>
      </div>
    </div>

    <!-- Filter & controls toolbar -->
    <Card>
      <CardContent class="space-y-3 p-4">
        <div class="flex flex-wrap items-center gap-2">
          <!-- Min Level -->
          <div class="flex items-center gap-1.5">
            <span class="text-xs font-medium text-muted-foreground">{{ t('system.logs.minLevel') }}:</span>
            <select
              :value="(filter.min_level ?? 'INFO').toUpperCase()"
              data-testid="logs-level-select"
              class="h-8 rounded-md border border-input bg-background px-2.5 text-xs font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              @change="
                emit('update:filter', {
                  min_level: ($event.target as HTMLSelectElement).value as LogLevel,
                })
              "
            >
              <option v-for="lvl in LOG_LEVELS" :key="lvl" :value="lvl">
                {{ lvl }}
              </option>
            </select>
          </div>

          <!-- Search box -->
          <div class="min-w-[220px] flex-1">
            <input
              :value="filter.q ?? ''"
              type="search"
              data-testid="logs-search-input"
              :placeholder="t('system.logs.searchPlaceholder')"
              class="flex h-8 w-full rounded-md border border-input bg-background px-3 py-1 text-xs shadow-xs transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              @input="
                emit('update:filter', {
                  q: ($event.target as HTMLInputElement).value,
                })
              "
            />
          </div>

          <!-- Stream actions -->
          <div class="flex flex-wrap items-center gap-1.5">
            <Button
              size="sm"
              :variant="paused ? 'default' : 'outline'"
              data-testid="logs-toggle-pause"
              class="h-8 gap-1.5 text-xs"
              @click="emit('toggle-pause')"
            >
              <Play v-if="paused" class="size-3.5" />
              <Pause v-else class="size-3.5" />
              <span>
                {{
                  paused
                    ? t('system.logs.resumeStream', { count: queuedCount })
                    : t('system.logs.pauseStream')
                }}
              </span>
            </Button>

            <Button
              v-if="demoMode"
              size="sm"
              variant="outline"
              data-testid="logs-simulate-demo"
              class="h-8 gap-1.5 text-xs"
              @click="emit('simulate-demo')"
            >
              <Sparkles class="size-3.5" />
              <span>{{ t('system.logs.simulateDemo') }}</span>
            </Button>

            <Button
              size="sm"
              variant="outline"
              data-testid="logs-export-jsonl"
              class="h-8 gap-1.5 text-xs"
              :disabled="entries.length === 0"
              @click="emit('export-jsonl')"
            >
              <Download class="size-3.5" />
              <span>{{ t('system.logs.exportJsonl') }}</span>
            </Button>

            <Button
              size="sm"
              variant="ghost"
              data-testid="logs-clear"
              class="h-8 gap-1.5 text-xs"
              @click="emit('clear-view')"
            >
              <Trash2 class="size-3.5" />
              <span>{{ t('system.logs.clearConsole') }}</span>
            </Button>
          </div>
        </div>

        <!-- Subsystem quick filter chips -->
        <div class="flex flex-wrap items-center gap-1.5" data-testid="logs-subsystem-chips">
          <button
            v-for="chip in SUBSYSTEM_CHIPS"
            :key="chip.key || 'all'"
            type="button"
            :data-testid="`subsystem-chip-${chip.key || 'all'}`"
            :class="[
              'inline-flex h-6 items-center rounded-full border px-2.5 text-[11px] font-medium transition-colors',
              (filter.subsystem ?? '') === chip.key
                ? 'border-primary bg-primary text-primary-foreground'
                : 'border-border bg-muted/40 text-muted-foreground hover:bg-muted hover:text-foreground',
            ]"
            @click="emit('update:filter', { subsystem: chip.key || undefined })"
          >
            {{ chip.labelKey ? t(chip.labelKey) : chip.label }}
          </button>
        </div>

        <!-- Active correlation pills -->
        <div
          v-if="hasCorrelationFilters"
          data-testid="logs-active-correlation"
          class="flex flex-wrap items-center gap-2 rounded-lg border border-primary/25 bg-primary/5 px-3 py-2 text-xs"
        >
          <Filter class="size-3.5 text-primary" />
          <span class="font-medium">{{ t('system.logs.activeCorrelation') }}</span>

          <span
            v-if="filter.session_id"
            data-testid="active-pill-session"
            class="inline-flex items-center gap-1 rounded-md bg-background px-2 py-0.5 font-mono text-[11px] shadow-xs"
          >
            session_id={{ filter.session_id }}
            <button
              type="button"
              class="text-muted-foreground hover:text-foreground"
              @click="emit('update:filter', { session_id: undefined })"
            >
              <X class="size-3" />
            </button>
          </span>

          <span
            v-if="filter.turn_id != null"
            data-testid="active-pill-turn"
            class="inline-flex items-center gap-1 rounded-md bg-background px-2 py-0.5 font-mono text-[11px] shadow-xs"
          >
            turn_id={{ filter.turn_id }}
            <button
              type="button"
              class="text-muted-foreground hover:text-foreground"
              @click="emit('update:filter', { turn_id: undefined })"
            >
              <X class="size-3" />
            </button>
          </span>

          <span
            v-if="filter.request_id"
            data-testid="active-pill-request"
            class="inline-flex items-center gap-1 rounded-md bg-background px-2 py-0.5 font-mono text-[11px] shadow-xs"
          >
            request_id={{ filter.request_id }}
            <button
              type="button"
              class="text-muted-foreground hover:text-foreground"
              @click="emit('update:filter', { request_id: undefined })"
            >
              <X class="size-3" />
            </button>
          </span>

          <span
            v-if="filter.task_id"
            data-testid="active-pill-task"
            class="inline-flex items-center gap-1 rounded-md bg-background px-2 py-0.5 font-mono text-[11px] shadow-xs"
          >
            task_id={{ filter.task_id }}
            <button
              type="button"
              class="text-muted-foreground hover:text-foreground"
              @click="emit('update:filter', { task_id: undefined })"
            >
              <X class="size-3" />
            </button>
          </span>

          <button
            type="button"
            data-testid="logs-clear-correlation"
            class="ml-auto text-xs font-medium text-primary underline-offset-4 hover:underline"
            @click="emit('clear-correlation')"
          >
            {{ t('system.logs.clearCorrelation') }}
          </button>
        </div>
      </CardContent>
    </Card>

    <!-- Lagged banner -->
    <div
      v-if="laggedCount > 0"
      role="status"
      data-testid="logs-lagged-banner"
      class="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-800 dark:text-amber-300"
    >
      {{ t('system.logs.laggedNotice', { count: laggedCount }) }}
    </div>

    <!-- Log console stream list + floating resume button -->
    <div class="relative">
      <div
        ref="consoleContainer"
        data-testid="logs-entry-list"
        class="max-h-[620px] overflow-y-auto rounded-xl border bg-zinc-950 font-mono text-xs text-zinc-100 divide-y divide-zinc-800/80"
        @scroll="onConsoleScroll"
      >
        <div
          v-if="entries.length === 0"
          data-testid="logs-empty"
          class="p-8 text-center font-sans text-sm text-zinc-400"
        >
          {{ loading ? t('common.loading') : t('system.logs.empty') }}
        </div>

        <div
          v-for="entry in entries"
          :key="entry.seq"
          :data-testid="`log-row-${entry.seq}`"
          class="transition-colors hover:bg-zinc-900/80"
        >
          <!-- Row summary -->
          <div
            class="flex cursor-pointer flex-wrap items-center gap-2 px-3 py-2"
            @click="toggleRow(entry.seq)"
          >
            <component
              :is="expandedSeqs.has(entry.seq) ? ChevronDown : ChevronRight"
              class="size-3.5 shrink-0 text-zinc-500"
            />
            <span class="shrink-0 text-[11px] text-zinc-400">
              {{ formatTimeMs(entry.timestamp_ms) }}
            </span>
            <Badge :variant="levelVariant(entry.level)" class="px-1.5 py-0 text-[10px]">
              {{ entry.level }}
            </Badge>
            <span class="rounded bg-zinc-800 px-1.5 py-0.5 text-[11px] text-sky-300">
              {{ entry.subsystem }}
            </span>
            <span v-if="entry.event" class="font-semibold text-emerald-300">
              {{ entry.event }}
            </span>
            <span class="min-w-[160px] flex-1 truncate text-zinc-200">
              {{ entry.message }}
            </span>

            <!-- Inline correlation badges -->
            <span
              v-if="entry.session_id"
              class="rounded bg-violet-950/80 px-1.5 py-0.5 text-[10px] text-violet-300"
            >
              {{ entry.session_id }}
            </span>
            <span
              v-if="entry.turn_id != null"
              class="rounded bg-indigo-950/80 px-1.5 py-0.5 text-[10px] text-indigo-300"
            >
              turn#{{ entry.turn_id }}
            </span>
            <span
              v-if="entry.duration_ms != null"
              class="rounded bg-zinc-800 px-1.5 py-0.5 text-[10px] text-amber-300"
            >
              {{ entry.duration_ms }}ms
            </span>
            <span
              v-if="entry.error_code"
              class="rounded bg-red-950/80 px-1.5 py-0.5 text-[10px] text-red-300"
            >
              {{ entry.error_code }}
            </span>
          </div>

          <!-- Expanded detail drawer -->
          <div
            v-if="expandedSeqs.has(entry.seq)"
            :data-testid="`log-detail-${entry.seq}`"
            class="space-y-2.5 border-t border-zinc-800/70 bg-zinc-900/60 px-4 py-3"
          >
            <!-- One-click correlation buttons -->
            <div class="flex flex-wrap items-center gap-2 font-sans">
              <Button
                v-if="entry.session_id"
                size="sm"
                variant="secondary"
                :data-testid="`filter-session-${entry.seq}`"
                class="h-7 text-xs"
                @click.stop="emit('update:filter', { session_id: entry.session_id })"
              >
                {{ t('system.logs.filterBySession', { id: entry.session_id }) }}
              </Button>

              <Button
                v-if="entry.turn_id != null"
                size="sm"
                variant="secondary"
                :data-testid="`filter-turn-${entry.seq}`"
                class="h-7 text-xs"
                @click.stop="
                  emit('update:filter', {
                    session_id: entry.session_id ?? undefined,
                    turn_id: entry.turn_id,
                  })
                "
              >
                {{ t('system.logs.filterByTurn', { id: entry.turn_id }) }}
              </Button>

              <Button
                v-if="entry.request_id"
                size="sm"
                variant="secondary"
                :data-testid="`filter-request-${entry.seq}`"
                class="h-7 text-xs"
                @click.stop="emit('update:filter', { request_id: entry.request_id })"
              >
                {{ t('system.logs.filterByRequest', { id: entry.request_id }) }}
              </Button>

              <Button
                v-if="entry.task_id"
                size="sm"
                variant="secondary"
                :data-testid="`filter-task-${entry.seq}`"
                class="h-7 text-xs"
                @click.stop="emit('update:filter', { task_id: entry.task_id })"
              >
                {{ t('system.logs.filterByTask', { id: entry.task_id }) }}
              </Button>
            </div>

            <div class="grid gap-1 text-[11px] text-zinc-300 sm:grid-cols-2 lg:grid-cols-4">
              <div><span class="text-zinc-500">seq:</span> {{ entry.seq }}</div>
              <div><span class="text-zinc-500">target:</span> {{ entry.target }}</div>
              <div v-if="entry.outcome"><span class="text-zinc-500">outcome:</span> {{ entry.outcome }}</div>
              <div v-if="entry.generation != null"><span class="text-zinc-500">generation:</span> {{ entry.generation }}</div>
              <div v-if="entry.agent_key"><span class="text-zinc-500">agent_key:</span> {{ entry.agent_key }}</div>
              <div v-if="entry.device_id != null"><span class="text-zinc-500">device_id:</span> {{ entry.device_id }}</div>
              <div v-if="entry.request_id"><span class="text-zinc-500">request_id:</span> {{ entry.request_id }}</div>
            </div>

            <pre
              v-if="entry.fields && Object.keys(entry.fields).length > 0"
              class="overflow-x-auto rounded-lg bg-zinc-950 p-2.5 text-[11px] text-zinc-200"
            >{{ JSON.stringify(entry.fields, null, 2) }}</pre>
          </div>
        </div>
      </div>

      <!-- Floating bottom banner when paused with queued entries -->
      <div
        v-if="paused && queuedCount > 0"
        class="pointer-events-none absolute inset-x-0 bottom-3 flex justify-center"
      >
        <Button
          size="sm"
          data-testid="logs-floating-resume"
          class="pointer-events-auto h-8 gap-1.5 rounded-full shadow-lg"
          @click="emit('toggle-pause')"
        >
          <ArrowDown class="size-3.5" />
          <span>{{ t('system.logs.resumeStream', { count: queuedCount }) }}</span>
        </Button>
      </div>
    </div>
  </div>
</template>

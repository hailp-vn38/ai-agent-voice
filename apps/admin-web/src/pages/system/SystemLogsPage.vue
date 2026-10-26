<script setup lang="ts">
import { ArrowLeft, RefreshCcw } from '@lucide/vue'
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { RouterLink } from 'vue-router'

import { adjustLevelCount, listenSystemLogStream, systemLogsApi } from '@/api/system-logs'
import type {
  AdminAuditEventItem,
  AdminAuditLogFilter,
  LogLevelSummary,
  LogQueryFilter,
  StructuredLogEntry,
} from '@/api/types/system-logs'
import PageHeader from '@/components/admin/PageHeader.vue'
import AdminAuditTable from '@/components/system/AdminAuditTable.vue'
import LogStreamConsole from '@/components/system/LogStreamConsole.vue'
import { Button } from '@/components/ui/button'
import { useI18n } from '@/composables/useI18n'

const MAX_VISIBLE_ROWS = 1000
const MAX_PAUSED_QUEUE = 500

const { t } = useI18n()

const activeTab = ref<'runtime' | 'audit'>('runtime')

const filter = ref<LogQueryFilter>({
  min_level: 'INFO',
  limit: 200,
})
const entries = ref<StructuredLogEntry[]>([])
const queuedEntries = ref<StructuredLogEntry[]>([])
const paused = ref(false)
const laggedCount = ref(0)
const demoMode = ref(false)
const loadingLogs = ref(false)
const bufferedCount = ref(0)
const capacity = ref(4000)
const droppedEvicted = ref(0)
const counts = ref<LogLevelSummary>({
  error: 0,
  warn: 0,
  info: 0,
  debug: 0,
  trace: 0,
})

const auditFilter = ref<AdminAuditLogFilter>({
  page: 1,
  page_size: 50,
})
const auditItems = ref<AdminAuditEventItem[]>([])
const auditTotal = ref(0)
const loadingAudit = ref(false)

let pageAbort = new AbortController()
let streamAbort: AbortController | null = null
let latestSeenSeq = 0

function appendEntriesDeduplicated(
  target: StructuredLogEntry[],
  incoming: StructuredLogEntry[],
  maxRows = MAX_VISIBLE_ROWS,
) {
  const existingSeqs = new Set(target.map((item) => item.seq))
  for (const item of incoming) {
    if (!existingSeqs.has(item.seq)) {
      target.push(item)
      existingSeqs.add(item.seq)
      if (item.seq > latestSeenSeq) {
        latestSeenSeq = item.seq
      }
    }
  }
  if (target.length > maxRows) {
    target.splice(0, target.length - maxRows)
  }
}

function incrementSummaryCount(entry: StructuredLogEntry) {
  adjustLevelCount(counts.value, entry.level, 1)
  bufferedCount.value = Math.min(capacity.value, bufferedCount.value + 1)
}

async function loadSnapshot() {
  loadingLogs.value = true
  try {
    const page = await systemLogsApi.list(filter.value, pageAbort.signal)
    if (pageAbort.signal.aborted) return
    entries.value = page.items
    latestSeenSeq = page.latest_seq
    bufferedCount.value = page.buffered_count
    capacity.value = page.capacity
    droppedEvicted.value = page.dropped_evicted
    counts.value = { ...page.counts }
    demoMode.value = Boolean(page.demo_mode)
  } finally {
    loadingLogs.value = false
  }
}

function startStream() {
  streamAbort?.abort()
  streamAbort = new AbortController()
  const currentSignal = streamAbort.signal

  void listenSystemLogStream(
    filter.value,
    {
      onReady(payload) {
        capacity.value = payload.capacity
        if (latestSeenSeq > 0 && payload.latest_seq > latestSeenSeq) {
          void resyncAfterLag(latestSeenSeq)
        } else if (payload.latest_seq > latestSeenSeq) {
          latestSeenSeq = payload.latest_seq
        }
      },
      onEntry(entry) {
        incrementSummaryCount(entry)
        if (paused.value) {
          appendEntriesDeduplicated(queuedEntries.value, [entry], MAX_PAUSED_QUEUE)
        } else {
          appendEntriesDeduplicated(entries.value, [entry], MAX_VISIBLE_ROWS)
        }
      },
      onLagged(payload) {
        laggedCount.value += payload.missed
        void resyncAfterLag(latestSeenSeq)
      },
    },
    currentSignal,
  )
}

async function resyncAfterLag(afterSeq: number) {
  try {
    const delta = await systemLogsApi.list(
      { ...filter.value, after_seq: afterSeq, limit: 500 },
      pageAbort.signal,
    )
    if (pageAbort.signal.aborted) return
    if (paused.value) {
      appendEntriesDeduplicated(queuedEntries.value, delta.items, MAX_PAUSED_QUEUE)
    } else {
      appendEntriesDeduplicated(entries.value, delta.items, MAX_VISIBLE_ROWS)
    }
    bufferedCount.value = delta.buffered_count
    droppedEvicted.value = delta.dropped_evicted
    counts.value = { ...delta.counts }
  } catch {
    // Keep existing stream view if resync fails.
  }
}

async function loadAuditLogs() {
  loadingAudit.value = true
  try {
    const page = await systemLogsApi.auditLogs(auditFilter.value, pageAbort.signal)
    if (pageAbort.signal.aborted) return
    auditItems.value = page.items
    auditTotal.value = page.total
  } finally {
    loadingAudit.value = false
  }
}

function updateFilter(patch: Partial<LogQueryFilter>) {
  filter.value = {
    ...filter.value,
    ...patch,
  }
  laggedCount.value = 0
  void loadSnapshot()
  startStream()
}

function clearCorrelation() {
  filter.value = {
    ...filter.value,
    session_id: undefined,
    turn_id: undefined,
    request_id: undefined,
    task_id: undefined,
  }
  void loadSnapshot()
  startStream()
}

function togglePause() {
  if (paused.value) {
    paused.value = false
    if (queuedEntries.value.length > 0) {
      appendEntriesDeduplicated(entries.value, queuedEntries.value, MAX_VISIBLE_ROWS)
      queuedEntries.value = []
    }
  } else {
    paused.value = true
  }
}

function clearView() {
  entries.value = []
  queuedEntries.value = []
  laggedCount.value = 0
}

function exportJsonl() {
  if (entries.value.length === 0) return
  const jsonl = entries.value.map((row) => JSON.stringify(row)).join('\n') + '\n'
  const blob = new Blob([jsonl], { type: 'application/x-ndjson;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = `system-logs-${Date.now()}.jsonl`
  anchor.click()
  URL.revokeObjectURL(url)
}

async function simulateDemo() {
  await systemLogsApi.simulateDemoTurnLogs()
}

function updateAuditFilter(patch: Partial<AdminAuditLogFilter>) {
  auditFilter.value = {
    ...auditFilter.value,
    ...patch,
  }
  void loadAuditLogs()
}

function selectTab(tab: 'runtime' | 'audit') {
  activeTab.value = tab
  if (tab === 'audit' && auditItems.value.length === 0) {
    void loadAuditLogs()
  }
}

function refreshCurrent() {
  if (activeTab.value === 'runtime') {
    void loadSnapshot()
    startStream()
  } else {
    void loadAuditLogs()
  }
}

onMounted(() => {
  void loadSnapshot()
  startStream()
  void loadAuditLogs()
})

onBeforeUnmount(() => {
  streamAbort?.abort()
  pageAbort.abort()
})
</script>

<template>
  <section class="space-y-6">
    <div>
      <RouterLink
        to="/system"
        class="inline-flex items-center gap-1.5 text-xs font-medium text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft class="size-3.5" />
        <span>{{ t('common.back') }}</span>
      </RouterLink>
    </div>

    <PageHeader
      :eyebrow="t('system.logs.eyebrow')"
      :title="t('system.logs.title')"
      :description="t('system.logs.description')"
    >
      <template #actions>
        <Button
          variant="outline"
          data-testid="logs-refresh"
          :disabled="loadingLogs || loadingAudit"
          @click="refreshCurrent"
        >
          <RefreshCcw :class="['size-4', (loadingLogs || loadingAudit) && 'animate-spin']" />
          {{ t('system.refresh') }}
        </Button>
      </template>
    </PageHeader>

    <!-- Tabs -->
    <div class="flex items-center gap-2 border-b pb-2">
      <Button
        size="sm"
        :variant="activeTab === 'runtime' ? 'default' : 'ghost'"
        data-testid="tab-runtime-logs"
        @click="selectTab('runtime')"
      >
        {{ t('system.logs.tabRuntime') }}
      </Button>
      <Button
        size="sm"
        :variant="activeTab === 'audit' ? 'default' : 'ghost'"
        data-testid="tab-audit-logs"
        @click="selectTab('audit')"
      >
        {{ t('system.logs.tabAudit') }} ({{ auditTotal }})
      </Button>
    </div>

    <LogStreamConsole
      v-if="activeTab === 'runtime'"
      :entries="entries"
      :counts="counts"
      :buffered-count="bufferedCount"
      :capacity="capacity"
      :dropped-evicted="droppedEvicted"
      :filter="filter"
      :paused="paused"
      :queued-count="queuedEntries.length"
      :lagged-count="laggedCount"
      :demo-mode="demoMode"
      :loading="loadingLogs"
      @update:filter="updateFilter"
      @clear-correlation="clearCorrelation"
      @toggle-pause="togglePause"
      @scroll-pause="paused = true"
      @clear-view="clearView"
      @export-jsonl="exportJsonl"
      @simulate-demo="simulateDemo"
    />

    <AdminAuditTable
      v-else
      :items="auditItems"
      :total="auditTotal"
      :filter="auditFilter"
      :loading="loadingAudit"
      @update:filter="updateAuditFilter"
    />
  </section>
</template>

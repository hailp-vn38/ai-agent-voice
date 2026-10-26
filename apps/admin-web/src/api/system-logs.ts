import { apiUrl, requestJson, withQuery } from '@/api/client'
import { ApiError, isApiError } from '@/api/errors'
import type {
  AdminAuditEventItem,
  AdminAuditLogFilter,
  AdminAuditLogPage,
  LogLevel,
  LogLevelSummary,
  LogQueryFilter,
  LogStreamLaggedEvent,
  LogStreamReadyEvent,
  StructuredLogEntry,
  SystemLogPage,
  SystemLogStreamCallbacks,
} from '@/api/types/system-logs'
import { readAdminToken } from '@/stores/auth'

const LEVEL_SEVERITY: Record<LogLevel, number> = {
  ERROR: 5,
  WARN: 4,
  INFO: 3,
  DEBUG: 2,
  TRACE: 1,
}

const DEMO_CAPACITY = 4000
const MAX_SSE_FRAME_BYTES = 16 * 1024
const BASE_TIME_MS = 1793000000000

export function adjustLevelCount(counts: LogLevelSummary, level: LogLevel, delta: number) {
  if (level === 'ERROR') counts.error += delta
  else if (level === 'WARN') counts.warn += delta
  else if (level === 'INFO') counts.info += delta
  else if (level === 'DEBUG') counts.debug += delta
  else if (level === 'TRACE') counts.trace += delta
}

function buildInitialDemoEntries(): StructuredLogEntry[] {
  return [
    {
      seq: 1,
      timestamp_ms: BASE_TIME_MS - 45_000,
      level: 'INFO',
      subsystem: 'system',
      event: 'server_listening',
      message: 'Voice agent server listening',
      target: 'voice_agent_server',
      outcome: 'success',
      fields: { bind_addr: '0.0.0.0:8000' },
    },
    {
      seq: 2,
      timestamp_ms: BASE_TIME_MS - 30_000,
      level: 'INFO',
      subsystem: 'voice.ws',
      event: 'ws_handshake_accepted',
      message: 'Accepted ESP32 WebSocket connection',
      target: 'voice_agent_server::app::transport::websocket',
      outcome: 'success',
      session_id: 'sess-demo-101',
      agent_key: 'home_assistant',
      fields: { protocol_version: 1 },
    },
    {
      seq: 3,
      timestamp_ms: BASE_TIME_MS - 28_500,
      level: 'INFO',
      subsystem: 'voice.asr',
      event: 'asr_final_recognized',
      message: 'ASR final hypothesis recognized',
      target: 'voice_agent_server::session::actor::listening',
      session_id: 'sess-demo-101',
      turn_id: 1,
      generation: 1,
      agent_key: 'home_assistant',
      fields: { text_chars: 28 },
    },
    {
      seq: 4,
      timestamp_ms: BASE_TIME_MS - 27_900,
      level: 'INFO',
      subsystem: 'voice.llm',
      event: 'llm_stream_completed',
      message: 'LLM stream completed',
      target: 'voice_agent_server::workers::llm',
      outcome: 'success',
      session_id: 'sess-demo-101',
      turn_id: 1,
      generation: 1,
      agent_key: 'home_assistant',
      duration_ms: 540,
      fields: { ttft_ms: 190, tool_calls: 0 },
    },
    {
      seq: 5,
      timestamp_ms: BASE_TIME_MS - 27_200,
      level: 'INFO',
      subsystem: 'voice.tts',
      event: 'tts_delivery_drained',
      message: 'TTS speech delivery drained cleanly',
      target: 'voice_agent_server::session::actor::delivery',
      outcome: 'completed',
      session_id: 'sess-demo-101',
      turn_id: 1,
      generation: 1,
      agent_key: 'home_assistant',
      duration_ms: 680,
      fields: {},
    },
    {
      seq: 6,
      timestamp_ms: BASE_TIME_MS - 15_000,
      level: 'WARN',
      subsystem: 'mcp.external',
      event: 'external_mcp_call',
      message: 'External MCP tool call exceeded soft latency threshold',
      target: 'voice_agent_server::tools::external_mcp',
      outcome: 'degraded',
      session_id: 'sess-demo-102',
      turn_id: 2,
      generation: 1,
      agent_key: 'office_concierge',
      duration_ms: 1850,
      fields: { server_key: 'calendar_bridge', tool_name: 'list_upcoming_events' },
    },
    {
      seq: 7,
      timestamp_ms: BASE_TIME_MS - 8_000,
      level: 'ERROR',
      subsystem: 'notification',
      event: 'notification_channel_failed',
      message: 'External notification channel delivery failed',
      target: 'voice_agent_server::services::notification_hub::dispatcher',
      outcome: 'failure',
      duration_ms: 412,
      error_code: 'http_401',
      fields: { channel: 'telegram', notification_id: 'notif-demo-1', attempts: 2 },
    },
    {
      seq: 8,
      timestamp_ms: BASE_TIME_MS - 3_000,
      level: 'INFO',
      subsystem: 'admin.api',
      event: 'provider_test_completed',
      message: 'Diagnostic provider test completed',
      target: 'voice_agent_server::app::admin::providers',
      outcome: 'success',
      request_id: 'req-demo-8801',
      duration_ms: 142,
      fields: { provider_key: 'openai_primary', kind: 'llm' },
    },
  ]
}

function buildInitialDemoAuditItems(): AdminAuditEventItem[] {
  const baseSec = Math.floor(BASE_TIME_MS / 1000)
  return [
    {
      id: 104,
      created_at: baseSec - 120,
      request_id: 'req-audit-104',
      resource_type: 'provider',
      resource_id: 1,
      action: 'update',
      prior_revision: 3,
      new_revision: 4,
      outcome: 'success',
      error_kind: null,
      affected_rows: 1,
    },
    {
      id: 103,
      created_at: baseSec - 900,
      request_id: 'req-audit-103',
      resource_type: 'agent_speaker_policy',
      resource_id: 2,
      action: 'update',
      prior_revision: 1,
      new_revision: 2,
      outcome: 'success',
      error_kind: null,
      affected_rows: 1,
    },
    {
      id: 102,
      created_at: baseSec - 2400,
      request_id: 'req-audit-102',
      resource_type: 'mcp_tool_approval',
      resource_id: 3,
      action: 'approve',
      prior_revision: 1,
      new_revision: 2,
      outcome: 'success',
      error_kind: null,
      affected_rows: 1,
    },
    {
      id: 101,
      created_at: baseSec - 5400,
      request_id: 'req-audit-101',
      resource_type: 'device',
      resource_id: 7,
      action: 'delete',
      prior_revision: 2,
      new_revision: null,
      outcome: 'conflict',
      error_kind: 'revision_conflict',
      affected_rows: 0,
    },
  ]
}

let demoEntries: StructuredLogEntry[] = buildInitialDemoEntries()
let demoAuditItems: AdminAuditEventItem[] = buildInitialDemoAuditItems()
let demoSeq = demoEntries.length
let demoTurnCounter = 10

interface DemoStreamSubscriber {
  filter: LogQueryFilter
  callbacks: SystemLogStreamCallbacks
}

const demoSubscribers = new Set<DemoStreamSubscriber>()

function shouldFallbackToDemo(cause: unknown): boolean {
  if (isApiError(cause)) {
    return cause.status === 404 || cause.status === 501 || cause.status === 502 || cause.status === 503
  }
  return cause instanceof TypeError
}

function normalizeLevel(raw: LogQueryFilter['min_level']): LogLevel | undefined {
  if (!raw) return undefined
  const upper = raw.toUpperCase() as LogLevel
  return upper in LEVEL_SEVERITY ? upper : undefined
}

function matchesLogFilter(entry: StructuredLogEntry, filter: LogQueryFilter): boolean {
  if (filter.after_seq != null && entry.seq <= filter.after_seq) {
    return false
  }
  const minLevel = normalizeLevel(filter.min_level)
  if (minLevel) {
    const minSev = LEVEL_SEVERITY[minLevel] ?? 3
    const entrySev = LEVEL_SEVERITY[entry.level] ?? 3
    if (entrySev < minSev) {
      return false
    }
  }
  if (filter.subsystem) {
    const sub = filter.subsystem.trim()
    if (
      sub &&
      entry.subsystem !== sub &&
      !entry.subsystem.startsWith(`${sub}.`)
    ) {
      return false
    }
  }
  if (filter.outcome && entry.outcome !== filter.outcome.trim()) {
    return false
  }
  if (filter.session_id && entry.session_id !== filter.session_id.trim()) {
    return false
  }
  if (filter.turn_id != null && entry.turn_id !== filter.turn_id) {
    return false
  }
  if (filter.request_id && entry.request_id !== filter.request_id.trim()) {
    return false
  }
  if (filter.task_id && entry.task_id !== filter.task_id.trim()) {
    return false
  }
  if (filter.agent_key && entry.agent_key !== filter.agent_key.trim()) {
    return false
  }
  if (filter.q) {
    const needle = filter.q.trim().toLowerCase()
    if (needle) {
      const haystacks = [
        entry.message,
        entry.event ?? '',
        entry.subsystem,
        entry.target,
        entry.error_code ?? '',
        entry.session_id ?? '',
        entry.request_id ?? '',
        entry.task_id ?? '',
        entry.agent_key ?? '',
        JSON.stringify(entry.fields ?? {}),
      ]
      if (!haystacks.some((h) => h.toLowerCase().includes(needle))) {
        return false
      }
    }
  }
  return true
}

function computeDemoCounts(entries: StructuredLogEntry[]): LogLevelSummary {
  const counts: LogLevelSummary = { error: 0, warn: 0, info: 0, debug: 0, trace: 0 }
  for (const item of entries) {
    adjustLevelCount(counts, item.level, 1)
  }
  return counts
}

function queryDemoLogs(filter: LogQueryFilter): SystemLogPage {
  const limit = Math.min(Math.max(filter.limit ?? 200, 1), 500)
  const filtered = demoEntries.filter((entry) => matchesLogFilter(entry, filter))
  const items =
    filter.after_seq != null
      ? filtered.slice(0, limit)
      : filtered.slice(Math.max(0, filtered.length - limit))
  return {
    items: items.map((entry) => ({ ...entry, fields: { ...entry.fields } })),
    oldest_seq: demoEntries[0]?.seq ?? null,
    latest_seq: demoSeq,
    buffered_count: demoEntries.length,
    capacity: DEMO_CAPACITY,
    dropped_evicted: 0,
    counts: computeDemoCounts(demoEntries),
    demo_mode: true,
  }
}

function queryDemoAuditLogs(filter: AdminAuditLogFilter): AdminAuditLogPage {
  const filtered = demoAuditItems.filter((item) => {
    if (filter.resource_type && item.resource_type !== filter.resource_type) return false
    if (filter.outcome && item.outcome !== filter.outcome) return false
    return true
  })
  const page = Math.max(filter.page ?? 1, 1)
  const pageSize = Math.min(Math.max(filter.page_size ?? 50, 1), 200)
  const offset = (page - 1) * pageSize
  return {
    items: filtered.slice(offset, offset + pageSize),
    total: filtered.length,
    page,
    page_size: pageSize,
    demo_mode: true,
  }
}

function toLogQueryRecord(filter: LogQueryFilter): Record<string, string | number | undefined> {
  return {
    after_seq: filter.after_seq,
    min_level: filter.min_level ? filter.min_level.toLowerCase() : undefined,
    subsystem: filter.subsystem || undefined,
    session_id: filter.session_id || undefined,
    turn_id: filter.turn_id,
    request_id: filter.request_id || undefined,
    agent_key: filter.agent_key || undefined,
    task_id: filter.task_id || undefined,
    outcome: filter.outcome || undefined,
    q: filter.q || undefined,
    limit: filter.limit,
  }
}

export const systemLogsApi = {
  async list(filter: LogQueryFilter = {}, signal?: AbortSignal): Promise<SystemLogPage> {
    try {
      const path = withQuery('/api/admin/system/logs', toLogQueryRecord(filter))
      return await requestJson<SystemLogPage>(path, {}, { signal })
    } catch (cause) {
      if (shouldFallbackToDemo(cause)) {
        return queryDemoLogs(filter)
      }
      throw cause
    }
  },

  async auditLogs(
    filter: AdminAuditLogFilter = {},
    signal?: AbortSignal,
  ): Promise<AdminAuditLogPage> {
    try {
      const path = withQuery('/api/admin/system/audit-logs', {
        page: filter.page,
        page_size: filter.page_size,
        resource_type: filter.resource_type || undefined,
        outcome: filter.outcome || undefined,
      })
      return await requestJson<AdminAuditLogPage>(path, {}, { signal })
    } catch (cause) {
      if (shouldFallbackToDemo(cause)) {
        return queryDemoAuditLogs(filter)
      }
      throw cause
    }
  },

  async simulateDemoTurnLogs(): Promise<StructuredLogEntry[]> {
    demoTurnCounter += 1
    const sessionId = `sess-demo-${200 + demoTurnCounter}`
    const turnId = demoTurnCounter
    const now = Date.now()
    const batch: StructuredLogEntry[] = [
      {
        seq: ++demoSeq,
        timestamp_ms: now,
        level: 'INFO',
        subsystem: 'voice.ws',
        event: 'ws_connected',
        message: 'ServerHello sent; voice session connected',
        target: 'voice_agent_server::app::transport::websocket',
        outcome: 'success',
        session_id: sessionId,
        agent_key: 'home_assistant',
        duration_ms: 18,
        fields: {},
      },
      {
        seq: ++demoSeq,
        timestamp_ms: now + 10,
        level: 'INFO',
        subsystem: 'voice.vad',
        event: 'vad_cycle_started',
        message: 'VAD speech capture cycle started',
        target: 'voice_agent_server::session::actor::listening',
        session_id: sessionId,
        turn_id: turnId,
        generation: 1,
        agent_key: 'home_assistant',
        fields: { cycle_id: 1 },
      },
      {
        seq: ++demoSeq,
        timestamp_ms: now + 320,
        level: 'INFO',
        subsystem: 'voice.asr',
        event: 'asr_final_recognized',
        message: 'ASR final hypothesis recognized',
        target: 'voice_agent_server::session::actor::listening',
        outcome: 'success',
        session_id: sessionId,
        turn_id: turnId,
        generation: 1,
        agent_key: 'home_assistant',
        fields: { text_chars: 36 },
      },
      {
        seq: ++demoSeq,
        timestamp_ms: now + 350,
        level: 'INFO',
        subsystem: 'voice.speaker',
        event: 'speaker_observe_completed',
        message: 'Speaker recognition observation finished',
        target: 'voice_agent_server::session::actor::observe',
        outcome: 'matched',
        session_id: sessionId,
        turn_id: turnId,
        generation: 1,
        agent_key: 'home_assistant',
        duration_ms: 28,
        fields: { speaker_key: 'owner_voice' },
      },
      {
        seq: ++demoSeq,
        timestamp_ms: now + 365,
        level: 'INFO',
        subsystem: 'voice.llm',
        event: 'llm_operation_started',
        message: 'LLM operation started',
        target: 'voice_agent_server::session::actor::delivery',
        session_id: sessionId,
        turn_id: turnId,
        generation: 1,
        agent_key: 'home_assistant',
        fields: { profile_source: 'template', tool_count: 3 },
      },
      {
        seq: ++demoSeq,
        timestamp_ms: now + 620,
        level: 'INFO',
        subsystem: 'mcp.external',
        event: 'external_mcp_call',
        message: 'External MCP tool call completed',
        target: 'voice_agent_server::tools::external_mcp',
        outcome: 'success',
        session_id: sessionId,
        turn_id: turnId,
        generation: 1,
        agent_key: 'home_assistant',
        duration_ms: 210,
        fields: { server_key: 'weather', tool_name: 'get_forecast' },
      },
      {
        seq: ++demoSeq,
        timestamp_ms: now + 980,
        level: 'INFO',
        subsystem: 'voice.tts',
        event: 'tts_synthesis_completed',
        message: 'TTS synthesis ended',
        target: 'voice_agent_server::workers::tts::pool',
        outcome: 'Finished',
        session_id: sessionId,
        turn_id: turnId,
        generation: 1,
        agent_key: 'home_assistant',
        duration_ms: 340,
        fields: {},
      },
    ]

    demoEntries.push(...batch)
    for (const entry of batch) {
      for (const sub of demoSubscribers) {
        if (matchesLogFilter(entry, sub.filter)) {
          sub.callbacks.onEntry({ ...entry, fields: { ...entry.fields } })
        }
      }
    }
    return batch
  },
}

export async function listenSystemLogStream(
  filter: LogQueryFilter,
  callbacks: SystemLogStreamCallbacks,
  signal: AbortSignal,
): Promise<void> {
  const demoSubscriber: DemoStreamSubscriber = { filter, callbacks }
  demoSubscribers.add(demoSubscriber)

  try {
    const token = readAdminToken()
    const path = withQuery('/api/admin/system/logs/stream', toLogQueryRecord(filter))

    const response = await fetch(apiUrl(path), {
      headers: {
        Accept: 'text/event-stream',
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
      },
      cache: 'no-store',
      signal,
    })

    if (response.status === 401 || response.status === 403) {
      callbacks.onError?.(
        new ApiError('Unauthorized log stream request', response.status, 'unauthorized'),
      )
      return
    }

    if (
      !response.ok ||
      !response.body ||
      !response.headers.get('content-type')?.startsWith('text/event-stream')
    ) {
      callbacks.onReady?.({ latest_seq: demoSeq, capacity: DEMO_CAPACITY })
      return
    }

    const reader = response.body.getReader()
    const decoder = new TextDecoder()
    let buffer = ''
    let eventName = ''
    let dataLines: string[] = []
    let frameBytes = 0

    const resetFrame = () => {
      eventName = ''
      dataLines = []
      frameBytes = 0
    }

    const flushEvent = () => {
      if (!eventName && dataLines.length === 0) return
      const rawData = dataLines.join('\n')
      try {
        if (eventName === 'ready') {
          const parsed = JSON.parse(rawData) as LogStreamReadyEvent
          callbacks.onReady?.(parsed)
        } else if (eventName === 'log_entry') {
          const parsed = JSON.parse(rawData) as StructuredLogEntry
          if (parsed && typeof parsed.seq === 'number') {
            callbacks.onEntry(parsed)
          }
        } else if (eventName === 'lagged') {
          const parsed = JSON.parse(rawData) as LogStreamLaggedEvent
          callbacks.onLagged?.(parsed)
        }
      } catch (err) {
        callbacks.onError?.(err)
      } finally {
        resetFrame()
      }
    }

    try {
      while (!signal.aborted) {
        const { done, value } = await reader.read()
        if (done) break
        buffer += decoder.decode(value, { stream: true }).replace(/\r\n?/g, '\n')
        if (buffer.length > MAX_SSE_FRAME_BYTES * 2) {
          buffer = ''
          resetFrame()
          callbacks.onError?.(new Error('SSE stream buffer exceeded 16 KiB frame limit'))
          continue
        }
        let newline: number
        while ((newline = buffer.indexOf('\n')) !== -1) {
          const line = buffer.slice(0, newline)
          buffer = buffer.slice(newline + 1)
          if (line === '') {
            flushEvent()
          } else {
            frameBytes += line.length
            if (frameBytes > MAX_SSE_FRAME_BYTES) {
              resetFrame()
              callbacks.onError?.(new Error('SSE frame exceeded 16 KiB limit'))
              continue
            }
            if (line.startsWith('event:')) {
              eventName = line.slice(6).trimStart()
            } else if (line.startsWith('data:')) {
              dataLines.push(line.slice(5).trimStart())
            }
          }
        }
      }
      if (eventName || dataLines.length > 0) {
        flushEvent()
      }
    } finally {
      reader.releaseLock()
    }
  } catch (err) {
    if (!signal.aborted) {
      callbacks.onError?.(err)
    }
  } finally {
    if (signal.aborted) {
      demoSubscribers.delete(demoSubscriber)
    } else {
      signal.addEventListener('abort', () => demoSubscribers.delete(demoSubscriber), { once: true })
    }
  }
}

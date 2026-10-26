export type LogLevel = 'ERROR' | 'WARN' | 'INFO' | 'DEBUG' | 'TRACE'

export const LOG_LEVELS: readonly LogLevel[] = ['ERROR', 'WARN', 'INFO', 'DEBUG', 'TRACE'] as const

export const CANONICAL_SUBSYSTEMS = [
  'voice.ws',
  'voice.vad',
  'voice.asr',
  'voice.speaker',
  'voice.llm',
  'voice.tts',
  'mcp.device',
  'mcp.external',
  'task_flow',
  'notification',
  'admin.api',
  'system',
] as const

export type CanonicalLogSubsystem = (typeof CANONICAL_SUBSYSTEMS)[number]

export interface StructuredLogEntry {
  seq: number
  timestamp_ms: number
  level: LogLevel
  subsystem: string
  event?: string | null
  message: string
  target: string
  outcome?: string | null
  session_id?: string | null
  turn_id?: number | null
  generation?: number | null
  request_id?: string | null
  task_id?: string | null
  agent_key?: string | null
  device_id?: number | null
  duration_ms?: number | null
  error_code?: string | null
  fields: Record<string, unknown>
}

export interface LogQueryFilter {
  after_seq?: number
  min_level?: LogLevel | Lowercase<LogLevel>
  subsystem?: string
  session_id?: string
  turn_id?: number
  request_id?: string
  agent_key?: string
  task_id?: string
  outcome?: string
  q?: string
  limit?: number
}

export interface LogLevelSummary {
  error: number
  warn: number
  info: number
  debug: number
  trace: number
}

export interface SystemLogPage {
  items: StructuredLogEntry[]
  oldest_seq: number | null
  latest_seq: number
  buffered_count: number
  capacity: number
  dropped_evicted: number
  counts: LogLevelSummary
  demo_mode?: boolean
}

export interface LogStreamReadyEvent {
  latest_seq: number
  capacity: number
}

export interface LogStreamLaggedEvent {
  missed: number
  latest_seq: number
}

export interface SystemLogStreamCallbacks {
  onReady?: (payload: LogStreamReadyEvent) => void
  onEntry: (entry: StructuredLogEntry) => void
  onLagged?: (payload: LogStreamLaggedEvent) => void
  onError?: (err: unknown) => void
}

export interface AdminAuditEventItem {
  id: number
  created_at: number
  request_id: string
  resource_type: string
  resource_id: number | null
  action: string
  prior_revision: number | null
  new_revision: number | null
  outcome: 'success' | 'conflict' | string
  error_kind: string | null
  affected_rows: number | null
}

export interface AdminAuditLogFilter {
  page?: number
  page_size?: number
  resource_type?: string
  outcome?: string
}

export interface AdminAuditLogPage {
  items: AdminAuditEventItem[]
  total: number
  page: number
  page_size: number
  demo_mode?: boolean
}

/** Frontend contract for proposed Admin Agent Job endpoints. Not the existing Reminder API. */
export interface BackgroundProviderOption {
  ref: string
  name: string
  source: 'config' | 'database'
}
export interface BackgroundSettings {
  llm_providers?: BackgroundProviderOption[]
  runtime_ready: boolean
  enabled: boolean
  orchestrator_llm_provider: string | null
  worker_llm_provider: string | null
  revision: number
}
export interface BackgroundSettingsUpdate {
  enabled: boolean
  orchestrator_llm_provider: string | null
  worker_llm_provider: string | null
}

export type PluginStatus = 'disabled' | 'ready' | 'error'
export interface PluginOperation {
  name: string
  description?: string
  replay?: 'safe_read'
}
export interface InstalledPlugin {
  id: string
  name: string
  description: string
  version: string
  digest?: string
  status: PluginStatus
  revision: number
  runtime: { kind: 'shared_process'; language: 'python' | 'rust' }
  operations: PluginOperation[]
  last_error_code?: string | null
  installation_digest?: string
}
export interface PluginTestResult {
  ok: boolean
  message?: string
  code?: string
}
export interface PluginPage {
  items: InstalledPlugin[]
  page: number
  page_size: number
  total?: number
}

export type JobStatus = 'queued' | 'planning' | 'running' | 'cancelling' | 'succeeded' | 'partial_success' | 'failed' | 'cancelled'
export type StepStatus = 'pending' | 'running' | 'succeeded' | 'failed' | 'skipped' | 'cancelled'
export interface JobStep {
  step_key: string
  plugin_id: string | null
  operation: string | null
  status: StepStatus
  error_code?: string | null
  attempt_state?: string | null
  instruction: string
  required: boolean
  dependencies: string[]
  input_refs: string[]
  arguments: Record<string, unknown>
  output_schema: Record<string, unknown>
  result: unknown
}
export interface JobArtifact {
  id: string
  media_type: string
  size_bytes: number
  sha256: string
}
export interface AdminJob {
  id: string
  goal: string
  status: JobStatus
  created_at: string
  requester_scope: string
  plan_revision: number
  retry_of_job_id: string | null
  result: Record<string, unknown> | null
  budget_used: Record<string, unknown>
  steps: JobStep[]
  artifacts: JobArtifact[]
  error_code?: string | null
}
export interface JobPage {
  items: AdminJob[]
  page: number
  page_size: number
  total?: number
}

export type JobEventType = 'plan_attempt' | 'plan_rejected' | 'plan_accepted' | 'step_started' | 'step_finished' | 'job_finished' | 'llm_run_started' | 'llm_provider_acquired' | 'llm_request_started' | 'llm_response_received' | 'llm_run_finished'
export interface JobEvent {
  sequence: number
  event_type: JobEventType
  created_at: string
  step_key: string | null
  payload: Record<string, unknown>
}
export interface JobEventPage {
  items: JobEvent[]
}

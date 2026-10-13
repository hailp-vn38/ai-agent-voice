/** Frontend contract for proposed Admin Agent Job endpoints. Not the existing Reminder API. */
export interface BackgroundProviderOption {
  ref: string
  name: string
  source: 'config' | 'database'
}
export interface BackgroundSettings {
  llm_providers?: BackgroundProviderOption[]
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
  runtime: { kind: 'isolated_worker'; language: 'python' }
  operations: PluginOperation[]
  permissions?: { network_hosts: string[]; scratch_storage: boolean }
  permissions_approved?: boolean
  last_error_code?: string | null
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

export type JobStatus = 'queued' | 'planning' | 'running' | 'cancelling' | 'succeeded' | 'partial' | 'failed' | 'cancelled'
export type StepStatus = 'pending' | 'running' | 'succeeded' | 'failed' | 'skipped' | 'cancelled'
export interface JobStep {
  step_key: string
  plugin_id: string
  operation: string
  status: StepStatus
  required: boolean
  error_code?: string | null
  artifact_ids?: string[]
}
export interface JobArtifact {
  id: string
  name: string
  media_type: string
  size_bytes?: number
}
export interface AdminJob {
  id: string
  goal: string
  status: JobStatus
  created_at: string
  updated_at?: string
  requester_label?: string
  steps?: JobStep[]
  artifacts?: JobArtifact[]
  result_summary?: string | null
  error_code?: string | null
  retry_of_job_id?: string | null
}
export interface JobPage {
  items: AdminJob[]
  page: number
  page_size: number
  total?: number
}

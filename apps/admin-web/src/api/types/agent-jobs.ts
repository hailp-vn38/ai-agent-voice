import type { KnowledgeCurrentState } from './knowledge-processing'
/** Frontend contract for proposed Admin Agent Job endpoints. Not the existing Reminder API. */
export interface BackgroundProviderOption {
  ref: string
  name: string
  source: 'config' | 'database'
  adapter?: string
  model?: string
  base_url?: string
  timeout_ms?: number
  max_tokens?: number
  context_limit_tokens?: number
  reasoning_effort?: 'low' | 'medium' | 'high' | null
  temperature_percent?: number | null
}
export interface BackgroundAgentConfig {
  id: string
  flow_id: 'daily' | 'wiki' | (string & {})
  parent_id?: string | null
  mode: 'primary' | 'subagent' | 'all'
  enabled: boolean
  provider_ref: string | null
  effective_provider_ref: string
  effective_model: string
  steps: number
  default_steps: number
  allowed_tools: string[]
  default_allowed_tools?: string[]
  available_tools: string[]
  allowed_skills: string[]
  default_allowed_skills?: string[]
  available_skills: string[]
  allowed_subagents: string[]
  description: string
  default_description: string
  system_prompt: string
  default_system_prompt: string
}
export interface BackgroundAgentConfigUpdate {
  enabled: boolean
  provider_ref?: string | null
  steps?: number
  allowed_tools?: string[]
  allowed_skills?: string[]
  description?: string
  system_prompt?: string
}
export interface BackgroundSettings {
  llm_providers?: BackgroundProviderOption[]
  runtime_ready: boolean
  enabled: boolean
  orchestrator_llm_provider: string | null
  worker_llm_provider: string | null
  revision: number
  agents?: BackgroundAgentConfig[]
}
export interface BackgroundSettingsUpdate {
  enabled: boolean
  orchestrator_llm_provider: string | null
  worker_llm_provider: string | null
  agents?: Record<string, BackgroundAgentConfigUpdate>
}

export type AgentRole = 'orchestrator' | 'research' | 'document' | 'general' | 'root' | 'source_collector' | 'wiki_ingestor' | 'wiki_curator' | (string & {})
export interface AgentMessageView {
  id: string
  sequence: number
  role: 'user' | 'assistant' | (string & {})
  text: string
  tool_call_ids?: string[]
}
export interface AgentRunView {
  agent_run_id: string
  parent_agent_run_id: string | null
  fork_group_id?: string | null
  role: AgentRole
  goal_excerpt: string
  result?: Record<string, unknown> | null
  messages?: AgentMessageView[]
  status: 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled' | 'interrupted'
  run_index: number
  llm_rounds: number
  tool_calls: number
  input_tokens: number | null
  output_tokens: number | null
  started_at: string | null
  finished_at: string | null
}
export interface CoverageView {
  basis: 'structured_provenance' | 'goal_only_unverified'
  required_sources_count: number
  verified_sources_count: number
  required_capabilities_count: number
  verified_capabilities_count: number
  missing_sources_count: number
}
export type JobStatus = 'queued' | 'planning' | 'running' | 'cancelling' | 'succeeded' | 'partial_success' | 'failed' | 'cancelled'
export type StepStatus = 'pending' | 'running' | 'succeeded' | 'failed' | 'skipped' | 'cancelled'
export interface JobStep {
  step_key: string
  agent_run_id?: string | null
  plugin_id: string | null
  operation: string | null
  status: StepStatus
  error_code?: string | null
  attempt_state?: string | null
  instruction: string
  assistant_message?: string | null
  required: boolean
  dependencies: string[]
  input_refs: string[]
  arguments: Record<string, unknown>
  output_schema: Record<string, unknown>
  result: unknown
}
export interface JobArtifact {
  id: string
  step_key: string | null
  filename: string
  media_type: string
  size_bytes: number
  sha256: string
  created_at: string
  name?: string
  mime_type?: string
  kind?: string
  role?: string
  producer_session_id?: string | null
  producer_agent_id?: string | null
  producer_tool_name?: string | null
  producer_invocation_id?: string | null
  download_url?: string
}
export interface AdminJob {
  knowledge_state?: KnowledgeCurrentState | null
  execution_mode?: 'dag_v1' | 'hierarchical_v2' | 'hierarchical_v3' | 'plugin_operation_v3' | 'knowledge_ingest_v3' | 'orchestrator_agent'
  agent_runs?: AgentRunView[]
  coverage?: CoverageView | null
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

export type JobEventType = 'plan_attempt' | 'plan_rejected' | 'plan_accepted' | 'step_started' | 'step_finished' | 'job_finished' | 'llm_run_started' | 'llm_provider_acquired' | 'llm_request_started' | 'llm_response_received' | 'llm_run_finished' | 'agent_run_created' | 'agent_run_started' | 'agent_decision_accepted' | 'agent_decision_rejected' | 'agent_run_finished' | 'join_completed' | 'knowledge_analysis_submitted' | 'knowledge_proposal_changed'
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

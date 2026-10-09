import type { Page, PageQuery } from './common'
import type { TemplateProviderType } from './templates'

export type ProviderRuntimeState = 'cold' | 'queued' | 'loading' | 'ready' | 'failed' | 'draining' | 'quarantined'
export interface ProviderRuntime {
  desired_revision: number
  desired_state: ProviderRuntimeState
  ready_revisions: number[]
  can_prepare: boolean
  failure_code: string | null
}

export interface CredentialMetadata {
  id: string
  masked_key: string
  key_version: number
  status: 'active'
}

export interface AdminProvider {
  credential?: CredentialMetadata | null
  id: number
  key: string
  name: string
  type: TemplateProviderType
  adapter: string
  /** Canonical JSON returned by the server; parse only where the UI needs a field. */
  config_json: string
  enabled: 0 | 1
  revision: number
  created_at: number
  updated_at: number
  /** Fallback deployment env var name; credential values are write-only. */
  credential_env: string | null
  runtime_status: 'not_loaded' | 'unavailable' | 'loaded'
  runtime_matches_desired: boolean
  requires_restart: boolean
  runtime?: ProviderRuntime
}

export interface ProviderListQuery extends PageQuery {
  enabled?: boolean
  q?: string
  type?: TemplateProviderType
  sort?: 'key' | '-key' | 'name' | '-name'
}

export interface ProviderPage extends Page<AdminProvider> {
  facets: Record<string, number>
}

export interface CreateProviderInput {
  api_key?: string
  name: string
  type: TemplateProviderType
  adapter: string
  config_json: Record<string, unknown>
}

export interface UpdateProviderInput {
  api_key?: string
  name?: string
  adapter?: string
  config_json?: Record<string, unknown>
  enabled?: boolean
}

export interface ProviderTemplate {
  key: string
  name: string
  provider_type: TemplateProviderType
  enabled: boolean
}

export interface ProviderTemplatePage extends Page<ProviderTemplate> {
  provider_key: string
  revision: number
}

export interface ProviderAdapter {
  adapter: string
  type: TemplateProviderType
  display_name?: string
  name?: string
  description?: string
  capabilities?: Record<string, unknown>
  config_schema?: ProviderConfigSchema
  secret_ref?: boolean
  supports_discovery?: boolean
}

export interface ProviderAdapterListResponse {
  items: ProviderAdapter[]
}

export interface ProviderConfigField {
  key: string
  label?: string
  description?: string
  type: 'string' | 'integer' | 'boolean' | 'select'
  required?: boolean
  minimum?: number
  maximum?: number
  max_length?: number
  enum_values?: Array<string | number>
  enum_source?: 'models' | 'voices' | 'languages'
  advanced?: boolean
}

export interface ProviderConfigSchema {
  fields?: ProviderConfigField[]
}

export interface ProviderAdapterDiscoverInput {
  selection: Record<string, unknown>
}

export interface VadDiagnosticResult {
  probability: number
  sample_range: unknown
  elapsed_ms: number
  [field: string]: unknown
}

export interface AsrDiagnosticInput {
  audio: Blob
}

export interface LlmDiagnosticInput {
  input: string
}

export interface TtsDiagnosticInput {
  text: string
  voice?: string
  language?: string
}

export interface ProviderPrepareResult {
  provider_key: string
  desired_revision: number
  runtime: ProviderRuntime
}

/** Credentials are write-only and live only in the open form. */
export interface ProviderTestDraft {
  type: TemplateProviderType
  adapter: string
  config_json: Record<string, unknown>
  api_key?: string
  saved_credential?: { key: string; expected_revision: number }
}
export interface ProviderTextTestResult {
  test_source?: 'draft' | 'saved'
  type: 'llm' | 'asr'
  status: 'success'
  result: { text: string; language?: string }
  metrics: { elapsed_ms: number; audio_duration_ms?: number; rtf?: number }
}

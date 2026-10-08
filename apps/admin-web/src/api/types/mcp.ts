export type McpAuthInput =
  | { type: 'none' }
  | { type: 'bearer' }
  | { type: 'header'; header_name: string }

/** Only auth metadata is persisted. Credential lookup is deployment-owned. */
export type McpAuthDisplay =
  | { type: 'none' }
  | { type: 'bearer' }
  | { type: 'header'; header_name: string }

export interface AdminMcpServer {
  key: string
  name: string
  transport: 'streamable_http'
  url: string
  headers: Record<string, string>
  /** Read-only deployment environment variable for bearer/header authentication. */
  credential_env: string | null
  auth: McpAuthDisplay
  connect_timeout_ms: number
  request_timeout_ms: number
  enabled: boolean
  revision: number
  created_at: number
  updated_at: number
}

export interface McpServerListQuery {
  page?: number
  pageSize?: number
  /** The current Rust API accepts but does not apply this filter. Filter client-side. */
  enabled?: boolean
}

export interface McpServerPage {
  items: AdminMcpServer[]
  page: number
  page_size: number
  max_page_size: number
  /** The Rust endpoint currently does not expose total. */
  total?: number
}

export interface CreateMcpServerInput {
  key: string
  name: string
  url: string
  auth: McpAuthInput
  connect_timeout_ms?: number
  request_timeout_ms?: number
}

export interface UpdateMcpServerInput {
  name?: string
  url?: string
  auth?: McpAuthInput
  connect_timeout_ms?: number
  request_timeout_ms?: number
  enabled?: boolean
}

export interface AgentMcpBinding {
  server_key: string
  enabled: boolean
  required: boolean
}

/** Actual GET /agents/{key}/mcp-bindings response: no revision envelope. */
export interface AgentMcpBindings {
  items: AgentMcpBinding[]
}

export interface PutAgentMcpBindingInput {
  enabled: boolean
  /** Unsupported by Rust server: always false. */
  required: false
}

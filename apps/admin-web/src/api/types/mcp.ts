export type McpAuthInput =
  | { type: 'none' }
  | { type: 'bearer'; secret_ref: string }
  | { type: 'header'; header_name: string; secret_ref: string }

/** Admin API redacts secret refs on reads. Never send this representation back as an auth mutation. */
export type McpAuthDisplay =
  | { type: 'none' }
  | { type: 'bearer'; has_secret_ref: boolean }
  | { type: 'header'; header_name: string; has_secret_ref: boolean }

export interface AdminMcpServer {
  key: string
  name: string
  transport: 'streamable_http'
  url: string
  headers: Record<string, string>
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
  headers?: Record<string, string>
  auth: McpAuthInput
  connect_timeout_ms?: number
  request_timeout_ms?: number
}

export interface UpdateMcpServerInput {
  name?: string
  url?: string
  headers?: Record<string, string>
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

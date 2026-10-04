import type { Page, PageQuery } from './common'

export type McpAuth =
  | { type: 'none' }
  | { type: 'bearer'; secret_ref: string }
  | { type: 'header'; header_name: string; secret_ref: string }

export interface AdminMcpServer {
  key: string
  name: string
  url: string
  headers: Record<string, string>
  auth: McpAuth
  connect_timeout_ms: number
  request_timeout_ms: number
  enabled: boolean
  revision: number
}

export interface McpServerListQuery extends PageQuery {
  enabled?: boolean
}

export interface CreateMcpServerInput {
  key: string
  name: string
  url: string
  headers?: Record<string, string>
  auth: McpAuth
  connect_timeout_ms?: number
  request_timeout_ms?: number
}

export interface UpdateMcpServerInput {
  name?: string
  url?: string
  headers?: Record<string, string>
  auth?: McpAuth
  connect_timeout_ms?: number
  request_timeout_ms?: number
  enabled?: boolean
}

export interface AgentMcpBinding {
  mcp_server_key: string
  enabled: boolean
  required: boolean
}

export interface AgentMcpBindings {
  agent_key: string
  revision: number
  bindings: AgentMcpBinding[]
}

export interface PutAgentMcpBindingInput {
  enabled: boolean
  /** The server currently rejects `true`; retained for an exact API body. */
  required: boolean
}

export type McpServerPage = Page<AdminMcpServer>

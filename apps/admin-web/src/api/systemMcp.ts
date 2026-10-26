import { jsonRequest, requestJson } from './client'

export interface SystemMcpGrants {
  system_read: boolean
  mcp_read: boolean
  mcp_discover: boolean
  mcp_manage_request: boolean
}
export interface SystemMcpSettings {
  enabled: boolean
  grants: SystemMcpGrants
  revision: number
}
export type ChangeCommand =
  | { tool: 'mcp_request_server_change'; server_key: string; enabled: boolean; expected_revision: number }
  | { tool: 'mcp_request_agent_binding_change'; agent_key: string; server_key: string; enabled: boolean; expected_agent_revision: number }
  | {
      tool: 'mcp_request_tool_review'
      agent_key: string
      server_key: string
      tool_name: string
      fingerprint: string
      observed_revision: number
      allow: boolean
      sensitive: boolean
      expected_revision: number
    }
export interface ChangeRequest {
  id: string
  command: ChangeCommand
  expires_at: number
  status: 'pending' | 'claimed' | 'applied' | 'failed' | 'rejected'
}

export type KnowledgeMcpAccess = 'read_only' | 'read_write'

export interface KnowledgeMcpToken {
  id: string
  name: string
  token: string
  access: KnowledgeMcpAccess
  created_at: number
}

export interface KnowledgeMcpToolInfo {
  name: string
  description: string
  access: KnowledgeMcpAccess
}

export interface KnowledgeMcpWikiStatus {
  total_documents?: number
  wiki_count?: number
  sources_count?: number
  total_links?: number
  [key: string]: unknown
}

export interface KnowledgeMcpState {
  revision: number
  enabled: boolean
  path: string
  base_url: string
  allow_write: boolean
  max_concurrency: number
  call_timeout_ms: number
  max_result_chars: number
  tokens: KnowledgeMcpToken[]
  tools: KnowledgeMcpToolInfo[]
  read_only_tools_count: number
  read_write_tools_count: number
  wiki_status?: KnowledgeMcpWikiStatus | null
  created_token?: KnowledgeMcpToken
}

export interface UpdateKnowledgeMcpInput {
  enabled: boolean
  allow_write: boolean
  max_concurrency?: number
  call_timeout_ms?: number
  max_result_chars?: number
}

export interface CreateKnowledgeMcpTokenInput {
  name: string
  access: KnowledgeMcpAccess
  custom_token?: string
}

const path = '/api/admin/system-mcp'

export const systemMcpApi = {
  settings: () => requestJson<SystemMcpSettings>(`${path}/grants`),
  save: (grants: SystemMcpGrants, revision: number) =>
    requestJson<SystemMcpSettings>(`${path}/grants`, jsonRequest('PUT', grants), { revision }),
  requests: () => requestJson<ChangeRequest[]>(`${path}/change-requests`),
  confirm: (id: string) =>
    requestJson<ChangeRequest>(`${path}/change-requests/${encodeURIComponent(id)}/confirm`, {
      method: 'POST',
    }),
  reject: (id: string) =>
    requestJson<ChangeRequest>(`${path}/change-requests/${encodeURIComponent(id)}/reject`, {
      method: 'POST',
    }),

  knowledge: (signal?: AbortSignal) =>
    requestJson<KnowledgeMcpState>(`${path}/knowledge`, { signal }),
  saveKnowledge: (input: UpdateKnowledgeMcpInput) =>
    requestJson<KnowledgeMcpState>(`${path}/knowledge`, jsonRequest('PUT', input)),
  createToken: (input: CreateKnowledgeMcpTokenInput) =>
    requestJson<KnowledgeMcpState>(`${path}/knowledge/tokens`, jsonRequest('POST', input)),
  deleteToken: (id: string) =>
    requestJson<KnowledgeMcpState>(`${path}/knowledge/tokens/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    }),
}

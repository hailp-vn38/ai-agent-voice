import { jsonRequest, requestJson } from './client'

export interface SystemMcpGrants {
  system_read: boolean
  mcp_read: boolean
  mcp_discover: boolean
  mcp_manage_request: boolean
}
export interface SystemMcpSettings { enabled: boolean; grants: SystemMcpGrants; revision: number }
export type ChangeCommand =
  | { tool: 'mcp_request_server_change'; server_key: string; enabled: boolean; expected_revision: number }
  | { tool: 'mcp_request_agent_binding_change'; agent_key: string; server_key: string; enabled: boolean; expected_agent_revision: number }
  | { tool: 'mcp_request_tool_review'; agent_key: string; server_key: string; tool_name: string; fingerprint: string; observed_revision: number; allow: boolean; sensitive: boolean; expected_revision: number }
export interface ChangeRequest { id: string; command: ChangeCommand; expires_at: number; status: 'pending' | 'claimed' | 'applied' | 'failed' | 'rejected' }
const path = '/api/admin/system-mcp'
export const systemMcpApi = {
  settings: () => requestJson<SystemMcpSettings>(`${path}/grants`),
  save: (grants: SystemMcpGrants, revision: number) => requestJson<SystemMcpSettings>(`${path}/grants`, jsonRequest('PUT', grants), { revision }),
  requests: () => requestJson<ChangeRequest[]>(`${path}/change-requests`),
  confirm: (id: string) => requestJson<ChangeRequest>(`${path}/change-requests/${encodeURIComponent(id)}/confirm`, { method: 'POST' }),
  reject: (id: string) => requestJson<ChangeRequest>(`${path}/change-requests/${encodeURIComponent(id)}/reject`, { method: 'POST' }),
}

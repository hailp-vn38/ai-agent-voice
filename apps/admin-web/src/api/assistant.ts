import { jsonRequest, requestJson } from './client'

export interface AssistantSettings {
  configured: boolean
  enabled: boolean
  device_id: string | null
  agent_key: string | null
  template_key: string | null
  revision: number | null
  active_connections: number
}
export interface AssistantTicket {
  device_id: string
  client_id: string
  ticket: string
  expires_in_seconds: number
}
export const assistantApi = {
  settings: (signal?: AbortSignal) => requestJson<AssistantSettings>('/api/admin/system/assistant-device', {}, { signal }),
  save: (input: { enabled: boolean; agent_key: string; template_key: string | null }, revision?: number) =>
    requestJson<AssistantSettings>('/api/admin/system/assistant-device', jsonRequest('PUT', input), { revision }),
  ticket: (signal?: AbortSignal) => requestJson<AssistantTicket>('/api/admin/assistant/ws-ticket', jsonRequest('POST', {}), { signal }),
}

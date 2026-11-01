import { jsonRequest, requestJson } from './client'
import type { MemoryMode } from './types/devices'

export interface AssistantSettings {
  configured: boolean
  enabled: boolean
  device_id: string | null
  agent_key: string | null
  template_key: string | null
  /** Absent until the Virtual Device is configured. */
  memory_mode?: MemoryMode
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
  save: (input: { enabled: boolean; agent_key: string; template_key: string | null; memory_mode?: MemoryMode }, revision?: number) =>
    requestJson<AssistantSettings>('/api/admin/system/assistant-device', jsonRequest('PUT', input), { revision }),
  ticket: (signal?: AbortSignal) => requestJson<AssistantTicket>('/api/admin/assistant/ws-ticket', jsonRequest('POST', {}), { signal }),
}

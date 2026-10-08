import { jsonRequest, requestJson } from './client'

export interface ObservedExternalTool {
  server_key: string
  original_name: string
  description: string
  input_schema: unknown
  fingerprint: string
  observed_revision: number
  observed_at: number
  revision: number
  allowed: boolean
  sensitive: boolean
  presence: 'observed_only'
  /** Server sends this; do not expose auth_reference/endpoint in the management UI. */
  source?: unknown
}

export interface ToolReviewInput {
  server_key: string
  original_name: string
  observed_revision: number
  fingerprint: string
  allowed: boolean
  sensitive: boolean
}

function allowlistPath(agentKey: string) {
  return `/api/admin/agents/${encodeURIComponent(agentKey)}/tool-allowlist`
}

export const externalToolsApi = {
  list(agentKey: string, signal?: AbortSignal) {
    return requestJson<{ items: ObservedExternalTool[] }>(allowlistPath(agentKey), {}, { signal })
  },
  review(agentKey: string, input: ToolReviewInput, revision: number) {
    return requestJson<{ revision: number }>(
      allowlistPath(agentKey),
      jsonRequest('PUT', input),
      { revision },
    )
  },
}

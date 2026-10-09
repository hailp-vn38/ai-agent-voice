import { jsonRequest, request, requestJson, withQuery } from './client'
import type {
  AdminMcpServer,
  McpProbeConfig, McpProbeResult, McpDiscoveryResult,
  CreateMcpServerInput,
  McpServerListQuery,
  McpServerPage,
  UpdateMcpServerInput,
} from './types/mcp'

const mcpServersPath = '/api/admin/mcp-servers'

function mcpServerPath(key: string) {
  return `${mcpServersPath}/${encodeURIComponent(key)}`
}

export const mcpApi = {
  testDraftConnection(server: McpProbeConfig, signal?: AbortSignal) {
    return requestJson<McpProbeResult>('/api/admin/mcp-tests/connection', jsonRequest('POST', { server }), { signal })
  },
  discoverDraftTools(server: McpProbeConfig, signal?: AbortSignal) {
    return requestJson<McpDiscoveryResult>('/api/admin/mcp-tests/discover', jsonRequest('POST', { server }), { signal })
  },
  testSavedConnection(key: string, signal?: AbortSignal) {
    return requestJson<McpProbeResult>(`${mcpServerPath(key)}/test/connection`, jsonRequest('POST', {}), { signal })
  },
  discoverSavedTools(key: string, signal?: AbortSignal) {
    return requestJson<McpDiscoveryResult>(`${mcpServerPath(key)}/test/discover`, jsonRequest('POST', {}), { signal })
  },
  list(query: McpServerListQuery = {}, signal?: AbortSignal) {
    return requestJson<McpServerPage>(withQuery(mcpServersPath, {
      page: query.page ?? 1,
      page_size: query.pageSize ?? 50,
      enabled: query.enabled,
    }), {}, { signal })
  },
  get(key: string, signal?: AbortSignal) {
    return requestJson<AdminMcpServer>(mcpServerPath(key), {}, { signal })
  },
  create(input: CreateMcpServerInput) {
    return requestJson<AdminMcpServer>(mcpServersPath, jsonRequest('POST', input, { allowHttpCredentials: true }))
  },
  update(key: string, input: UpdateMcpServerInput, revision: number) {
    return requestJson<AdminMcpServer>(mcpServerPath(key), jsonRequest('PATCH', input, { allowHttpCredentials: true }), { revision })
  },
  async remove(key: string, revision: number) {
    await request(mcpServerPath(key), { method: 'DELETE' }, { revision })
  },
}

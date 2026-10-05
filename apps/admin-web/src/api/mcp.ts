import { jsonRequest, request, requestJson, withQuery } from './client'
import type {
  AdminMcpServer,
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
    return requestJson<AdminMcpServer>(mcpServersPath, jsonRequest('POST', input))
  },
  update(key: string, input: UpdateMcpServerInput, revision: number) {
    return requestJson<AdminMcpServer>(mcpServerPath(key), jsonRequest('PATCH', input), { revision })
  },
  async remove(key: string, revision: number) {
    await request(mcpServerPath(key), { method: 'DELETE' }, { revision })
  },
}
